//! Run the mobile Git inspection against the saved account. Usage: git_probe <dir>
use jcode_mobile_core::AccountStore;
fn main() {
    let account = AccountStore::default_location().load().expect("no account");
    let dir = std::env::args().nth(1).unwrap_or_else(|| "/root".into());
    match jcode_mobile_core::git_info::inspect(&account, &dir) {
        Ok(s) => println!("{s:#?}"),
        Err(e) => println!("ERR: {e}"),
    }
}
