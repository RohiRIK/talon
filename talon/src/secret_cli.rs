//! `talon secret` — builtin vault CLI (spec criteria 1–3, 9).
//!
//! Values are entered via hidden prompt and printed only with an explicit
//! `--reveal`. The vault unlocks through the standard chain (keychain →
//! `TALON_MASTER_KEY` → passphrase prompt); a locked vault is an actionable
//! error, never a silent fallback.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use talon_memory::Database;
use talon_secrets::{
    BuiltinVault, Credential, ENV_VAR, MasterKey, MasterKeyStore, OsKeychain, SecretError,
    SecretProvider, SecretRef,
};

#[derive(Subcommand)]
pub enum SecretAction {
    /// Store a secret in the builtin vault (value via hidden prompt).
    Set {
        /// Secret name, referenced from jobs as {{secret:NAME}}.
        name: String,
    },
    /// Fetch a secret. Prints the value only with --reveal.
    Get {
        name: String,
        /// Print the plaintext value to stdout.
        #[arg(long)]
        reveal: bool,
    },
    /// List secret names and metadata — never values.
    List,
    /// Delete a secret from the builtin vault.
    Rm { name: String },
    /// Add or rotate an unlock wrap without re-encrypting stored secrets.
    Rewrap {
        /// Add/rotate the passphrase recovery wrap (prompts).
        #[arg(long)]
        passphrase: bool,
        /// (Re-)add the OS keychain wrap.
        #[arg(long)]
        keychain: bool,
    },
}

pub async fn run(action: SecretAction, talon_home: PathBuf) -> Result<()> {
    let keychain = OsKeychain;
    let key_store = MasterKeyStore::new(&talon_home, &keychain);

    // Criterion 2 headless path: TALON_MASTER_KEY alone is a valid unlock
    // source even when no wrapped copy exists on this machine.
    if !key_store.is_bootstrapped()? && std::env::var(ENV_VAR).is_err() {
        bail!(
            "no vault master key exists yet — run `talon init` to set up the vault \
             (the key is created only after you choose an unlock credential), \
             or set {ENV_VAR} for headless use"
        );
    }

    let master = unlock(&key_store)?;

    match action {
        SecretAction::Rewrap {
            passphrase,
            keychain: add_keychain,
        } => {
            if !passphrase && !add_keychain {
                bail!("choose at least one wrap: --passphrase and/or --keychain");
            }
            if passphrase {
                let pass = prompt_new_passphrase()?;
                key_store.rewrap_passphrase(&master, &pass)?;
                println!("Passphrase wrap written — stored secrets were not re-encrypted.");
            }
            if add_keychain {
                key_store.rewrap_keychain(&master)?;
                println!("Keychain wrap written.");
            }
            Ok(())
        }
        other => {
            let vault = open_vault(&talon_home, master).await?;
            run_vault_action(other, &vault).await
        }
    }
}

async fn run_vault_action(action: SecretAction, vault: &BuiltinVault) -> Result<()> {
    match action {
        SecretAction::Set { name } => {
            let value = inquire::Password::new(&format!("Value for `{name}`:"))
                .without_confirmation()
                .with_display_mode(inquire::PasswordDisplayMode::Masked)
                .prompt()
                .context("hidden prompt requires an interactive terminal")?;
            if value.is_empty() {
                bail!("refusing to store an empty value");
            }
            vault.set(&name, &value).await?;
            println!("Stored `{name}` (encrypted). Reference it as {{{{secret:{name}}}}}.");
            Ok(())
        }
        SecretAction::Get { name, reveal } => {
            if !reveal {
                bail!(
                    "`talon secret get` prints nothing without --reveal (values never print by accident)"
                );
            }
            let sref = SecretRef::parse(&format!("{{{{secret:{name}}}}}"))?;
            let value = vault.get(&sref).await?;
            println!("{}", value.expose());
            Ok(())
        }
        SecretAction::List => {
            let metas = vault.list().await?;
            if metas.is_empty() {
                println!("No secrets stored.");
                return Ok(());
            }
            println!("{:<32} {:<10} CREATED", "NAME", "PROVIDER");
            for m in metas {
                println!(
                    "{:<32} {:<10} {}",
                    m.name,
                    m.scheme,
                    m.created_at.as_deref().unwrap_or("-")
                );
            }
            Ok(())
        }
        SecretAction::Rm { name } => {
            let removed = vault.delete(&name).await?;
            if removed {
                println!("Deleted `{name}`.");
            } else {
                println!("No secret named `{name}`.");
            }
            Ok(())
        }
        SecretAction::Rewrap { .. } => unreachable!("handled in run()"),
    }
}

/// Standard unlock chain; falls back to a passphrase prompt only when a
/// recovery blob exists and we have a TTY.
fn unlock(key_store: &MasterKeyStore<'_>) -> Result<MasterKey> {
    let env_value = std::env::var(ENV_VAR).ok();
    match key_store.unlock(env_value.as_deref(), None) {
        Ok(key) => Ok(key),
        Err(SecretError::Locked { hint }) if key_store.recovery_blob_path().exists() => {
            let pass = inquire::Password::new("Vault passphrase:")
                .without_confirmation()
                .with_display_mode(inquire::PasswordDisplayMode::Masked)
                .prompt()
                .with_context(|| format!("vault is locked: {hint}"))?;
            Ok(key_store.unlock(env_value.as_deref(), Some(&pass))?)
        }
        Err(e) => Err(e.into()),
    }
}

async fn open_vault(talon_home: &Path, master: MasterKey) -> Result<BuiltinVault> {
    let db_path = talon_home.join("talon.db");
    let db_path = db_path
        .to_str()
        .context("talon home path is not valid UTF-8")?;
    let db = Arc::new(Database::open(db_path).map_err(|e| anyhow::anyhow!("open talon.db: {e}"))?);
    db.init_schema()
        .await
        .map_err(|e| anyhow::anyhow!("run migrations: {e}"))?;
    Ok(BuiltinVault::new(db, master))
}

fn prompt_new_passphrase() -> Result<String> {
    let pass = inquire::Password::new("New vault passphrase:")
        .with_display_mode(inquire::PasswordDisplayMode::Masked)
        .prompt()
        .context("passphrase prompt requires an interactive terminal")?;
    if pass.len() < 8 {
        bail!("passphrase must be at least 8 characters");
    }
    Ok(pass)
}

/// Interactive credential step for `talon init` (criterion 1): the credential
/// is chosen BEFORE any key material exists; non-interactive environments get
/// instructions instead of a silently generated key.
pub fn init_vault_bootstrap(talon_home: &Path) -> Result<()> {
    let keychain = OsKeychain;
    let key_store = MasterKeyStore::new(talon_home, &keychain);

    if key_store.is_bootstrapped()? {
        println!("Vault master key already set up — leaving it untouched.");
        return Ok(());
    }

    println!(
        "\nSecret vault setup — the master key is generated only after you pick an unlock credential."
    );

    let use_keychain = match inquire::Confirm::new("Store the master key in the OS keychain?")
        .with_default(true)
        .prompt()
    {
        Ok(v) => v,
        Err(e) => {
            // Non-interactive (CI/piped): skip, with the headless instructions.
            println!(
                "Skipped vault setup (non-interactive): rerun `talon init` in a terminal, \
                 or set {ENV_VAR} (base64 of a 32-byte key) for headless use."
            );
            tracing::warn!("vault bootstrap skipped: {e}");
            return Ok(());
        }
    };

    let add_passphrase = inquire::Confirm::new(
        "Also add a passphrase recovery wrap? (unlocks the vault if the keychain entry is lost)",
    )
    .with_default(!use_keychain)
    .prompt()
    .unwrap_or(false);

    let mut credentials = Vec::new();
    if use_keychain {
        credentials.push(Credential::Keychain);
    }
    if add_passphrase {
        credentials.push(Credential::Passphrase(prompt_new_passphrase()?));
    }
    if credentials.is_empty() {
        println!(
            "No credential chosen — vault not initialized. Rerun `talon init`, or set {ENV_VAR} for headless use."
        );
        return Ok(());
    }

    key_store.bootstrap(&credentials)?;
    println!("Vault master key created (stored only in wrapped form).");
    Ok(())
}
