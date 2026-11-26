-- Foundation schema: identity, access control, audit and settings.
-- Migrations are forward-only and named by timestamp so parallel work never collides.

create table users (
    id            uuid primary key default gen_random_uuid(),
    email         text        not null,
    display_name  text        not null default '',
    password_hash text        not null,
    disabled_at   timestamptz,
    created_at    timestamptz not null default now(),
    updated_at    timestamptz not null default now()
);
create unique index users_email_lower_idx on users (lower(email));

create table projects (
    id          uuid primary key default gen_random_uuid(),
    name        text        not null,
    slug        text        not null unique check (slug ~ '^[a-z0-9]([a-z0-9-]{0,38}[a-z0-9])?$'),
    description text        not null default '',
    created_by  uuid references users (id) on delete set null,
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now()
);

-- A membership grants a role at one scope. A global role applies to every cluster and project.
-- scope_id for 'cluster' points at a table that arrives in phase 2, so it has no foreign key.
create table memberships (
    id         uuid primary key default gen_random_uuid(),
    user_id    uuid        not null references users (id) on delete cascade,
    scope      text        not null check (scope in ('global', 'cluster', 'project')),
    scope_id   uuid,
    role       text        not null check (role in ('owner', 'admin', 'developer', 'viewer')),
    created_at timestamptz not null default now(),
    check ((scope = 'global') = (scope_id is null))
);
create unique index memberships_unique_idx
    on memberships (user_id, scope, coalesce(scope_id, '00000000-0000-0000-0000-000000000000'::uuid));
create index memberships_scope_idx on memberships (scope, scope_id);

create table setup_tokens (
    id         uuid primary key default gen_random_uuid(),
    token_hash bytea       not null unique,
    created_at timestamptz not null default now(),
    used_at    timestamptz
);

-- Browser sessions. A 'pending_2fa' session only proves the password and can do nothing else.
create table sessions (
    id               uuid primary key default gen_random_uuid(),
    token_hash       bytea       not null unique,
    user_id          uuid        not null references users (id) on delete cascade,
    csrf_token       text        not null,
    stage            text        not null check (stage in ('pending_2fa', 'active')),
    created_at       timestamptz not null default now(),
    expires_at       timestamptz not null,
    last_verified_at timestamptz not null,
    user_agent       text
);
create index sessions_user_idx on sessions (user_id);
create index sessions_expires_idx on sessions (expires_at);

create table settings (
    key        text primary key,
    value      jsonb       not null,
    updated_at timestamptz not null default now()
);

-- Records which master-key ids exist and holds a canary, so a wrong key file fails at startup.
create table encryption_keys (
    key_id            text primary key,
    canary_ciphertext bytea       not null,
    created_at        timestamptz not null default now()
);

create table user_totp (
    user_id           uuid primary key references users (id) on delete cascade,
    secret_ciphertext bytea       not null,
    key_id            text        not null,
    confirmed_at      timestamptz,
    last_used_step    bigint      not null default 0,
    created_at        timestamptz not null default now()
);

create table recovery_codes (
    id         uuid primary key default gen_random_uuid(),
    user_id    uuid        not null references users (id) on delete cascade,
    code_hash  text        not null,
    used_at    timestamptz,
    created_at timestamptz not null default now()
);
create index recovery_codes_user_idx on recovery_codes (user_id);

create table api_tokens (
    id           uuid primary key default gen_random_uuid(),
    user_id      uuid        not null references users (id) on delete cascade,
    name         text        not null,
    token_hash   bytea       not null unique,
    token_hint   text        not null,
    role         text        not null check (role in ('admin', 'developer', 'viewer')),
    project_ids  uuid[]      not null,
    expires_at   timestamptz not null,
    last_used_at timestamptz,
    revoked_at   timestamptz,
    created_at   timestamptz not null default now(),
    check (cardinality(project_ids) > 0)
);
create index api_tokens_user_idx on api_tokens (user_id);

-- Append-only record of who did what, including requests that were refused.
create table audit_events (
    id             bigint generated always as identity primary key,
    occurred_at    timestamptz not null default now(),
    actor_user_id  uuid,
    actor_token_id uuid,
    actor_ip       text,
    action         text        not null,
    scope          text        not null,
    scope_id       uuid,
    resource_type  text,
    resource_id    text,
    outcome        text        not null check (outcome in ('success', 'denied')),
    reason         text,
    metadata       jsonb       not null default '{}'::jsonb
);
create index audit_events_occurred_idx on audit_events (occurred_at desc);
create index audit_events_scope_idx on audit_events (scope, scope_id, occurred_at desc);
