use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

const SCHEMA: &str = "
create table if not exists users (
  id            integer primary key,
  username      text not null unique,
  display_name  text not null,
  password_hash text not null,
  bio           text not null default '',
  avatar_path   text,
  banner_path   text,
  created_at    integer not null
);

create table if not exists sessions (
  token_hash text primary key,
  user_id    integer not null references users(id) on delete cascade,
  created_at integer not null,
  expires_at integer not null
);

create table if not exists guilds (
  id   integer primary key,
  name text not null
);

create table if not exists channels (
  id       integer primary key,
  guild_id integer not null references guilds(id) on delete cascade,
  name     text not null,
  kind     text not null check (kind in ('text', 'voice'))
);

create table if not exists members (
  guild_id integer not null references guilds(id) on delete cascade,
  user_id  integer not null references users(id) on delete cascade,
  role     text not null check (role in ('admin', 'member')),
  primary key (guild_id, user_id)
);

create table if not exists messages (
  id         integer primary key,
  channel_id integer not null references channels(id) on delete cascade,
  author_id  integer not null references users(id),
  body       text not null,
  created_at integer not null
);

create index if not exists sessions_user_id on sessions(user_id);
create index if not exists messages_channel_id on messages(channel_id, id);
create index if not exists channels_guild_id on channels(guild_id);
";

pub async fn init_db(url: &str) -> Result<SqlitePool, sqlx::Error> {
    let pool = SqlitePoolOptions::new().max_connections(4).connect(url).await?;
    sqlx::raw_sql(SCHEMA).execute(&pool).await?;
    Ok(pool)
}

pub async fn init_memory_db() -> Result<SqlitePool, sqlx::Error> {
    let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;
    sqlx::raw_sql(SCHEMA).execute(&pool).await?;
    Ok(pool)
}
