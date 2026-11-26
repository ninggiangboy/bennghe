// SPDX-License-Identifier: AGPL-3.0-only
//! One-time recovery codes for accounts that lost their authenticator.
use sqlx::PgConnection;
use uuid::Uuid;

use super::password;
use crate::util::random_bytes;

pub const COUNT: usize = 10;
const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

/// `xxxxx-xxxxx`: about 49 bits, enough for a code that is rate limited and single use.
pub fn generate() -> Vec<String> {
    (0..COUNT)
        .map(|_| {
            let bytes = random_bytes::<10>();
            let chars: String = bytes.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect();
            format!("{}-{}", &chars[..5], &chars[5..])
        })
        .collect()
}

pub fn normalise(input: &str) -> String {
    input.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

/// Replaces all of a user's recovery codes with the given ones.
pub async fn replace_all(tx: &mut PgConnection, user_id: Uuid, codes: &[String]) -> anyhow::Result<()> {
    sqlx::query!("delete from recovery_codes where user_id = $1", user_id).execute(&mut *tx).await?;
    for code in codes {
        let hash = password::hash(normalise(code)).await?;
        sqlx::query!("insert into recovery_codes (user_id, code_hash) values ($1, $2)", user_id, hash).execute(&mut *tx).await?;
    }
    Ok(())
}

/// Spends the code if it matches an unused one. Each code works exactly once.
pub async fn consume(tx: &mut PgConnection, user_id: Uuid, input: &str) -> anyhow::Result<bool> {
    let normalised = normalise(input);
    if normalised.len() != 10 {
        return Ok(false);
    }
    let rows = sqlx::query!("select id, code_hash from recovery_codes where user_id = $1 and used_at is null", user_id)
        .fetch_all(&mut *tx)
        .await?;
    for row in rows {
        if password::verify(normalised.clone(), row.code_hash).await {
            let spent = sqlx::query!("update recovery_codes set used_at = now() where id = $1 and used_at is null", row.id)
                .execute(&mut *tx)
                .await?;
            return Ok(spent.rows_affected() == 1);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_ten_distinct_formatted_codes() {
        let codes = generate();
        assert_eq!(codes.len(), COUNT);
        for c in &codes {
            assert_eq!(c.len(), 11);
            assert_eq!(c.as_bytes()[5], b'-');
        }
        let mut sorted = codes.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), COUNT);
    }

    #[test]
    fn normalise_ignores_case_and_separators() {
        assert_eq!(normalise("AbC12-dE345"), "abc12de345");
    }
}
