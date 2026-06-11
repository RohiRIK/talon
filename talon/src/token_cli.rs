//! `talon token` — named API token management (criterion 4).
//!
//! The raw token prints exactly once, at creation. `list` shows metadata
//! only; revocation is a tombstone (the name stays visible for audit).

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use talon_memory::{Database, TokenRole, TokenStore};

#[derive(Subcommand)]
pub enum TokenAction {
    /// Create a named token; prints it once — store it somewhere safe.
    Create {
        /// Token name (1-64 chars), e.g. "laptop", "ci", "dashboard-ro".
        name: String,
        /// Access level: admin (full) or viewer (read-only).
        #[arg(long, default_value = "admin")]
        role: String,
    },
    /// List tokens — names, roles, usage; never values.
    List,
    /// Revoke a token by name (immediate; the name stays for audit).
    Revoke { name: String },
}

pub async fn run(action: TokenAction, talon_home: PathBuf) -> Result<()> {
    let db_path = talon_home.join("talon.db");
    let db_path = db_path
        .to_str()
        .context("talon home path is not valid UTF-8")?;
    let db = Arc::new(
        Database::open(db_path).map_err(|e| anyhow::anyhow!("open talon.db: {e}"))?,
    );
    db.init_schema()
        .await
        .map_err(|e| anyhow::anyhow!("run migrations: {e}"))?;
    let store = TokenStore::new(db);

    match action {
        TokenAction::Create { name, role } => {
            let name = name.trim();
            if name.is_empty() || name.len() > 64 {
                bail!("token name must be 1-64 characters");
            }
            let role = TokenRole::from_str(&role)
                .map_err(|_| anyhow::anyhow!("role must be `admin` or `viewer`"))?;
            let raw = store
                .create(name, role)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("Token `{name}` created ({}).", role.as_str());
            println!("\n  {raw}\n");
            println!("Store it now — it is never shown again.");
            Ok(())
        }
        TokenAction::List => {
            let metas = store.list().await.map_err(|e| anyhow::anyhow!("{e}"))?;
            if metas.is_empty() {
                println!("No tokens. Create one: talon token create NAME --role admin|viewer");
                return Ok(());
            }
            println!("{:<24} {:<8} {:<20} {:<20} STATUS", "NAME", "ROLE", "CREATED", "LAST USED");
            for m in metas {
                println!(
                    "{:<24} {:<8} {:<20} {:<20} {}",
                    m.name,
                    m.role.as_str(),
                    m.created_at,
                    m.last_used.as_deref().unwrap_or("-"),
                    if m.revoked { "revoked" } else { "active" }
                );
            }
            Ok(())
        }
        TokenAction::Revoke { name } => {
            if store
                .revoke(&name)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?
            {
                println!("Revoked `{name}`.");
            } else {
                println!("No active token named `{name}`.");
            }
            Ok(())
        }
    }
}
