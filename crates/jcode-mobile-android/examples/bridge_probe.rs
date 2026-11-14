//! Host-side adapter probe. Reads JSON commands on stdin and prints snapshots.
//! This uses the exact same Bridge as JNI, without requiring a JVM.
use std::io::{self, BufRead};

fn main() {
    let mut bridge = jcode_mobile_android::Bridge::default();
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        println!("{}", bridge.request(&line));
    }
}
