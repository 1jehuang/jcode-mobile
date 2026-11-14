//! Sign in with Apple and account linking against jcode.sh.
//!
//! Flow: [`new_nonce`] makes a raw nonce, the platform sheet is given
//! [`sha256_hex`] of it, and [`sign_in`] sends Apple's identity token plus the
//! raw nonce to `POST /auth/apple`, which verifies the token and returns an
//! account key exactly like email-code verification.
//!
//! Linking (Settings, "Link existing Jcode account"): [`start_link`] emails a
//! code to an existing account's address and [`verify_link`] merges the Apple
//! identity into that account so its subscription carries over. The current
//! key keeps working, so linking never interrupts the app.

use crate::cloud::{Account, CLIENT_NAME, DEFAULT_API_BASE};

/// Result of a successful link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Linked {
    pub email: Option<String>,
    /// True when the Apple identity moved into a different (existing) account.
    pub merged: bool,
}

/// 64 hex chars of OS randomness (two v4 UUIDs, 244 random bits).
pub fn new_nonce() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// Exchange an Apple identity token for a jcode.sh account key.
pub fn sign_in(identity_token: &str, raw_nonce: &str) -> Result<Account, String> {
    sign_in_at(DEFAULT_API_BASE, identity_token, raw_nonce)
}

pub fn sign_in_at(api_base: &str, identity_token: &str, raw_nonce: &str) -> Result<Account, String> {
    let body = post(
        api_base,
        "/auth/apple",
        None,
        serde_json::json!({ "identity_token": identity_token, "nonce": raw_nonce, "client_name": CLIENT_NAME }),
    )?;
    let api_key = body["api_key"].as_str().ok_or("jcode.sh returned no account key")?.to_string();
    Ok(Account {
        api_key,
        email: body["email"].as_str().map(str::to_string),
        given_name: body["given_name"].as_str().map(str::to_string),
        name: body["name"].as_str().map(str::to_string),
        api_base: api_base.trim_end_matches('/').into(),
    })
}

/// Mail a 6-digit code to `email`. Returns the link token for [`verify_link`].
pub fn start_link(account: &Account, email: &str) -> Result<String, String> {
    let body = post(&account.api_base, "/account/link/email/start", Some(account), serde_json::json!({ "email": email }))?;
    body["link_token"].as_str().map(str::to_string).ok_or_else(|| "jcode.sh returned no link token".into())
}

pub fn verify_link(account: &Account, link_token: &str, code: &str) -> Result<Linked, String> {
    let body = post(
        &account.api_base,
        "/account/link/email/verify",
        Some(account),
        serde_json::json!({ "link_token": link_token, "code": code }),
    )?;
    Ok(Linked { email: body["email"].as_str().map(str::to_string), merged: body["merged"].as_bool().unwrap_or(false) })
}

fn post(api_base: &str, path: &str, auth: Option<&Account>, json: serde_json::Value) -> Result<serde_json::Value, String> {
    let url = format!("{}{path}", api_base.trim_end_matches('/'));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .http_status_as_error(false)
        .user_agent(CLIENT_NAME)
        .build()
        .into();
    let mut req = agent.post(&url);
    if let Some(a) = auth {
        req = req.header("Authorization", &format!("Bearer {}", a.api_key));
    }
    let mut resp = req.send_json(json).map_err(|e| format!("Could not reach jcode.sh: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    let body: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    if (200..300).contains(&status) {
        return Ok(body);
    }
    let message = body["error"]["message"].as_str().unwrap_or_default();
    Err(if message.is_empty() { format!("jcode.sh error (HTTP {status})") } else { capitalize(message) })
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Lowercase hex SHA-256 (what Apple puts in the token's `nonce` claim).
pub fn sha256_hex(input: &str) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = input.as_bytes().to_vec();
    let bit_len = (msg.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(sha256_hex(""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            sha256_hex(&"a".repeat(1000)),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }

    #[test]
    fn nonce_is_fresh_hex() {
        let (a, b) = (new_nonce(), new_nonce());
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
