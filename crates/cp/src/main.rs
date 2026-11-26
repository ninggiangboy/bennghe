// SPDX-License-Identifier: AGPL-3.0-only
use std::path::PathBuf;

use anyhow::Context;
use bn_cp::{
    audit::{self, Actor, Entry},
    auth::{self, session},
    config::Config,
    secrets::{Secrets, SigningKeyFile},
    server, store,
};
use clap::{Parser, Subcommand};
use serde_json::json;

#[derive(Parser)]
#[command(name = "controlplane", version, about = "Bennghe control plane")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the control plane (the default).
    Serve,
    /// Create master.key and signing.key in the data directory. Never overwrites existing keys.
    Init {
        /// Defaults to BN_DATA_DIR, then /var/lib/bennghe.
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Print a new one-time token that authorises creating the owner account.
    SetupToken,
    /// Print the OpenAPI document as YAML.
    Openapi,
    /// Recovery commands that run on the host with access to the key files.
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
}

#[derive(Subcommand)]
enum AdminCommand {
    /// Remove two-factor authentication from an account whose owner lost every device and code.
    Reset2fa {
        #[arg(long)]
        user: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn".into()))
        .init();
    // rustls needs a process-wide crypto provider; ring keeps the binary small and musl-friendly.
    let _ = rustls::crypto::ring::default_provider().install_default();

    match Cli::parse().command.unwrap_or(Command::Serve) {
        Command::Serve => server::serve(Config::load()?).await,
        Command::Init { data_dir } => init(data_dir),
        Command::SetupToken => {
            let config = Config::load()?;
            let pool = store::connect(&config.database_url).await?;
            store::migrate(&pool).await?;
            if !auth::setup::needs_setup(&pool).await? {
                anyhow::bail!("this installation already has users; setup is finished");
            }
            println!("{}", auth::setup::create_token(&pool).await?);
            Ok(())
        }
        Command::Openapi => {
            print!("{}", bn_cp::api::openapi().to_yaml()?);
            Ok(())
        }
        Command::Admin { command: AdminCommand::Reset2fa { user } } => reset_2fa(&user).await,
    }
}

fn init(data_dir: Option<PathBuf>) -> anyhow::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let dir = data_dir.or_else(|| std::env::var_os("BN_DATA_DIR").map(PathBuf::from)).unwrap_or_else(|| "/var/lib/bennghe".into());
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let master = dir.join("master.key");
    let signing = dir.join("signing.key");
    if master.exists() {
        println!("kept existing {}", master.display());
    } else {
        Secrets::generate_file(&master)?;
        println!("created {}", master.display());
    }
    if signing.exists() {
        println!("kept existing {}", signing.display());
    } else {
        SigningKeyFile::generate(&signing)?;
        println!("created {}", signing.display());
    }
    println!("\nBack up master.key now. Without it every stored secret is unrecoverable.");
    Ok(())
}

async fn reset_2fa(email: &str) -> anyhow::Result<()> {
    let config = Config::load()?;
    let pool = store::connect(&config.database_url).await?;
    let email = email.trim().to_lowercase();
    let user = sqlx::query_scalar!("select id from users where lower(email) = $1", email)
        .fetch_optional(&pool)
        .await?
        .with_context(|| format!("no user with email {email}"))?;
    let mut tx = pool.begin().await?;
    sqlx::query!("delete from user_totp where user_id = $1", user).execute(&mut *tx).await?;
    sqlx::query!("delete from recovery_codes where user_id = $1", user).execute(&mut *tx).await?;
    let actor = Actor { user_id: None, token_id: None, ip: Some("host-cli".into()) };
    audit::record(&mut tx, &actor, &Entry::new("user.totp_reset", "global").resource("user", user).meta(json!({ "via": "host_cli" })))
        .await?;
    tx.commit().await?;
    session::delete_all_for_user(&pool, user).await?;
    println!("two-factor authentication removed for {email}; they must enrol again at next sign-in");
    Ok(())
}
