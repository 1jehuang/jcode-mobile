//! Inspect the signed-in mobile account's cloud machine without printing secrets.
//! Uses the same saved mobile account and bounded pinned-SSH inspection as Settings.
use jcode_mobile_core::{AccountStore, cloud_machine};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let account = AccountStore::default_location()
        .load()
        .ok_or("Sign in through Jcode Mobile first. No saved mobile account was found.")?;
    let snapshot = cloud_machine::inspect(&account).map_err(std::io::Error::other)?;
    println!("Home: {:?}", snapshot.home);
    println!("GitHub CLI installed: {}", snapshot.github_cli);
    println!(
        "GitHub authenticated now: {}",
        snapshot.github_authenticated
    );
    println!(
        "GitHub configuration present: {}",
        snapshot.github_config_present
    );
    println!("Possible SSH key files: {}", snapshot.ssh_key_count);
    println!(
        "Git credential file present: {}",
        snapshot.git_credentials_present
    );
    println!("Home entries (names only):");
    for entry in snapshot.entries {
        println!("  {entry:?}");
    }
    println!("Credential contents hidden. Restart/replacement persistence is not verified.");
    Ok(())
}
