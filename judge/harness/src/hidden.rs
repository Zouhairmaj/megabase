//! Held-out judge suite.
//!
//! Concrete cases are not stored in git. A secret seed expands a public
//! Level 1 grammar, and an optional sealed blob (a GitHub Actions
//! environment secret) carries human-authored cases. Summaries published
//! from this module are aggregate counts only.
//!
//! Sealing is ChaCha20 (RFC 8439) with HMAC-SHA256 in encrypt-then-MAC.
//! ChaCha20 alone does not authenticate ciphertext, so a failed MAC
//! rejects the blob before decryption. Keys are HKDF-SHA256 outputs of
//! the seed, domain-separated by `info`.

use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use base64::Engine;
use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::ChaCha20;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::case::{self, Case, Key, Step};
use crate::db;
use crate::run;

/// Environment variable that holds the seed. Never pass the seed as an argument.
pub const SEED_ENV: &str = "MEGABASE_JUDGE_HIDDEN_SEED";

/// Environment variable that holds the sealed human-authored cases, if any.
pub const CASES_ENV: &str = "MEGABASE_JUDGE_HIDDEN_CASES";

/// How many grammar cases one seed expands. Stable for a harness revision.
pub const GENERATED_CASE_COUNT: usize = 21;

const SEED_MIN_BYTES: usize = 32;
const MAX_PLAINTEXT: usize = 1024 * 1024;
const SALT: &[u8] = b"megabase-judge-hidden-v1";
const MAGIC: &[u8; 4] = b"MBJH";
const VERSION: u8 = 1;
const NONCE_LEN: usize = 12;
const MAC_LEN: usize = 32;
const WITHHELD: &str = "hidden suite: details are withheld";

type HmacSha256 = Hmac<Sha256>;

/// Seed material for the held-out suite. Debug output is redacted.
pub struct HiddenSeed {
    bytes: Vec<u8>,
}

impl std::fmt::Debug for HiddenSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HiddenSeed([redacted])")
    }
}

impl HiddenSeed {
    /// Parses trimmed seed bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the trimmed value is shorter than 32 bytes.
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.len() < SEED_MIN_BYTES {
            bail!("hidden seed must be at least {SEED_MIN_BYTES} bytes");
        }
        Ok(Self {
            bytes: trimmed.as_bytes().to_vec(),
        })
    }

    /// Reads [`SEED_ENV`] when `name` is that variable, or any named variable in tests.
    ///
    /// # Errors
    ///
    /// Returns an error when the variable is missing, empty, or too short.
    pub fn from_env(name: &str) -> Result<Self> {
        match std::env::var(name) {
            Ok(raw) if !raw.trim().is_empty() => Self::parse(&raw),
            _ => bail!("{name} is not set"),
        }
    }
}

/// Generated cases plus any cases opened from a sealed blob.
pub struct Assembled {
    pub cases: Vec<Case>,
    pub generated: usize,
    pub stored: usize,
}

/// Aggregate result. This is the only hidden-suite record that may be published.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HiddenReport {
    pub schema: u32,
    pub suite: String,
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub generated: u32,
    pub stored: u32,
}

struct Rng {
    cipher: ChaCha20,
}

impl Rng {
    fn from_seed(seed: &HiddenSeed) -> Result<Self> {
        let mut okm = [0u8; 44];
        expand(&seed.bytes, b"megabase-judge-hidden-rng-v1", &mut okm)?;
        let key: [u8; 32] = okm[..32].try_into().expect("32-byte key");
        let nonce: [u8; 12] = okm[32..].try_into().expect("12-byte nonce");
        Ok(Self {
            cipher: ChaCha20::new(&key.into(), &nonce.into()),
        })
    }

    fn fill(&mut self, buf: &mut [u8]) {
        buf.fill(0);
        self.cipher.apply_keystream(buf);
    }

    fn next_u32(&mut self) -> u32 {
        let mut buf = [0u8; 4];
        self.fill(&mut buf);
        u32::from_le_bytes(buf)
    }

    fn gen_range(&mut self, n: u32) -> u32 {
        assert!(n > 0, "range must be non-empty");
        let zone = u32::MAX - (u32::MAX % n);
        loop {
            let x = self.next_u32();
            if x < zone {
                return x % n;
            }
        }
    }

    fn gen_bool(&mut self) -> bool {
        self.gen_range(2) == 1
    }
}

fn expand(seed: &[u8], info: &[u8], okm: &mut [u8]) -> Result<()> {
    let hk = Hkdf::<Sha256>::new(Some(SALT), seed);
    hk.expand(info, okm)
        .map_err(|_| anyhow::anyhow!("HKDF expand failed for the hidden-suite key"))
}

fn hmac_key(seed: &HiddenSeed, info: &[u8]) -> Result<[u8; 32]> {
    let mut okm = [0u8; 32];
    expand(&seed.bytes, info, &mut okm)?;
    Ok(okm)
}

fn shuffle<T>(rng: &mut Rng, items: &mut [T]) {
    for i in (1..items.len()).rev() {
        let j = rng.gen_range((i + 1) as u32) as usize;
        items.swap(i, j);
    }
}

/// Seals `plaintext` for `seed`. The result is standard base64 with no case contents.
///
/// # Errors
///
/// Returns an error when the seed is unusable or `plaintext` exceeds 1 MiB.
pub fn seal(seed: &HiddenSeed, plaintext: &[u8]) -> Result<String> {
    if plaintext.len() > MAX_PLAINTEXT {
        bail!("hidden case plaintext exceeds {MAX_PLAINTEXT} bytes");
    }
    let enc_key = hmac_key(seed, b"megabase-judge-hidden-enc-v1")?;
    let mac_key = hmac_key(seed, b"megabase-judge-hidden-mac-v1")?;
    let mut mac = HmacSha256::new_from_slice(&mac_key).expect("32-byte MAC key");
    mac.update(b"nonce");
    mac.update(plaintext);
    let nonce_full = mac.finalize().into_bytes();
    let nonce: [u8; NONCE_LEN] = nonce_full[..NONCE_LEN].try_into().expect("nonce");

    let mut body = plaintext.to_vec();
    ChaCha20::new(&enc_key.into(), &nonce.into()).apply_keystream(&mut body);

    let mut message = Vec::with_capacity(5 + NONCE_LEN + body.len());
    message.extend_from_slice(MAGIC);
    message.push(VERSION);
    message.extend_from_slice(&nonce);
    message.extend_from_slice(&body);

    let mut tag = HmacSha256::new_from_slice(&mac_key).expect("32-byte MAC key");
    tag.update(&message);
    let tag = tag.finalize().into_bytes();
    message.extend_from_slice(&tag);
    Ok(base64::engine::general_purpose::STANDARD.encode(message))
}

/// Opens a blob produced by [`seal`].
///
/// # Errors
///
/// Returns an error when the blob is empty, too large, or fails authentication.
/// The error does not include plaintext.
pub fn open(seed: &HiddenSeed, blob: &str) -> Result<Vec<u8>> {
    let blob = blob.trim();
    if blob.is_empty() {
        bail!("hidden case blob is empty");
    }
    let raw = base64::engine::general_purpose::STANDARD
        .decode(blob)
        .context("hidden case blob is not base64")?;
    let min = MAGIC.len() + 1 + NONCE_LEN + MAC_LEN;
    if raw.len() < min || raw.len() > MAX_PLAINTEXT + min {
        bail!("hidden case blob has an invalid length");
    }
    let (message, tag) = raw.split_at(raw.len() - MAC_LEN);
    let mac_key = hmac_key(seed, b"megabase-judge-hidden-mac-v1")?;
    let mut mac = HmacSha256::new_from_slice(&mac_key).expect("32-byte MAC key");
    mac.update(message);
    mac.verify_slice(tag)
        .map_err(|_| anyhow::anyhow!("hidden case blob failed authentication"))?;
    if message.len() < min - MAC_LEN || &message[..4] != MAGIC || message[4] != VERSION {
        bail!("hidden case blob is not a supported sealed payload");
    }
    let nonce: [u8; NONCE_LEN] = message[5..5 + NONCE_LEN].try_into().expect("nonce");
    let mut body = message[5 + NONCE_LEN..].to_vec();
    let enc_key = hmac_key(seed, b"megabase-judge-hidden-enc-v1")?;
    ChaCha20::new(&enc_key.into(), &nonce.into()).apply_keystream(&mut body);
    Ok(body)
}

fn unseal_cases(seed: &HiddenSeed, blob: &str) -> Result<Vec<Case>> {
    let plain = open(seed, blob)?;
    let text = std::str::from_utf8(&plain).context("hidden case blob is not utf-8")?;
    let cases = case::parse_toml(text, "sealed cases")
        .map_err(|_| anyhow::anyhow!("hidden case blob is not a case file"))?;
    if cases.is_empty() {
        bail!("hidden case blob contains no cases");
    }
    Ok(cases)
}

/// Builds the in-memory suite. `sealed` is the base64 blob, not a path.
///
/// # Errors
///
/// Returns an error when the blob does not authenticate or the case set is invalid.
/// Those errors do not include case ids, paths, or bodies.
pub fn assemble(seed: &HiddenSeed, sealed: Option<&str>) -> Result<Assembled> {
    let mut rng = Rng::from_seed(seed)?;
    let mut cases = generate(&mut rng);
    let generated = cases.len();
    let stored = if let Some(blob) = sealed {
        let extra = unseal_cases(seed, blob)?;
        let n = extra.len();
        cases.extend(extra);
        n
    } else {
        0
    };
    shuffle(&mut rng, &mut cases);
    case::validate(&cases).map_err(|_| anyhow::anyhow!(WITHHELD))?;
    if cases.is_empty() {
        bail!("hidden suite produced no cases");
    }
    Ok(Assembled {
        cases,
        generated,
        stored,
    })
}

fn token(rng: &mut Rng, n: usize) -> String {
    const ALPH: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..n)
        .map(|_| ALPH[rng.gen_range(ALPH.len() as u32) as usize] as char)
        .collect()
}

fn case_id(rng: &mut Rng) -> String {
    let mut buf = [0u8; 8];
    rng.fill(&mut buf);
    let mut id = String::from("h");
    for byte in buf {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}

fn email(rng: &mut Rng) -> String {
    format!("h{}-{{{{run}}}}@example.com", token(rng, 10))
}

fn password(rng: &mut Rng) -> String {
    format!("A1a{}", token(rng, 12))
}

fn weak_password(rng: &mut Rng) -> String {
    let n = 1 + rng.gen_range(4);
    (0..n)
        .map(|_| char::from(b'0' + rng.gen_range(10) as u8))
        .collect()
}

fn toml_string(value: &str) -> toml::Value {
    toml::Value::String(value.to_string())
}

fn object(pairs: Vec<(&str, toml::Value)>) -> toml::Value {
    let mut table = toml::Table::new();
    for (key, value) in pairs {
        table.insert(key.to_string(), value);
    }
    toml::Value::Table(table)
}

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn http(
    rng: &mut Rng,
    units: &[&str],
    method: &str,
    path: String,
    key: Key,
    json: Option<toml::Value>,
    header_pairs: &[(&str, &str)],
    capture: &[(&str, &str)],
) -> Case {
    Case {
        id: case_id(rng),
        units: units.iter().map(|unit| (*unit).to_string()).collect(),
        description: String::new(),
        step: vec![Step {
            method: method.to_string(),
            path,
            key,
            headers: headers(header_pairs),
            json,
            body: None,
            compare_headers: Vec::new(),
            ignore: Vec::new(),
            capture: headers(capture),
        }],
        db: Vec::new(),
        snapshot: None,
    }
}

fn push_step(case: &mut Case, step: Step) {
    case.step.push(step);
}

fn one_step(
    method: &str,
    path: String,
    key: Key,
    json: Option<toml::Value>,
    header_pairs: &[(&str, &str)],
    capture: &[(&str, &str)],
) -> Step {
    Step {
        method: method.to_string(),
        path,
        key,
        headers: headers(header_pairs),
        json,
        body: None,
        compare_headers: Vec::new(),
        ignore: Vec::new(),
        capture: headers(capture),
    }
}

fn fixture_ids(rng: &mut Rng) -> String {
    let mut ids = Vec::new();
    for id in 1..=3 {
        if rng.gen_bool() {
            ids.push(id.to_string());
        }
    }
    if ids.is_empty() {
        ids.push("1".into());
    }
    ids.join(",")
}

fn select_list(rng: &mut Rng) -> String {
    const COLS: [&str; 4] = ["id", "title", "done", "priority"];
    let mut picked = Vec::new();
    let mask = 1 + rng.gen_range(15);
    for (index, col) in COLS.iter().enumerate() {
        if mask & (1 << index) != 0 {
            picked.push(*col);
        }
    }
    shuffle(rng, &mut picked);
    picked.join(",")
}

/// Read filters always include `id=in.(1,2,3)` when the predicate is not
/// already an `id` predicate, so rows inserted by an earlier run cannot
/// make the two databases disagree on a later read.
fn filter_query(rng: &mut Rng, kind: u32) -> (&'static str, String) {
    let bool_txt = if rng.gen_bool() { "true" } else { "false" };
    match kind % 12 {
        0 => ("eq", format!("done=eq.{bool_txt}&id=in.(1,2,3)")),
        1 => ("neq", format!("done=neq.{bool_txt}&id=in.(1,2,3)")),
        2 => (
            "gt",
            format!("priority=gt.{}&id=in.(1,2,3)", rng.gen_range(5)),
        ),
        3 => (
            "gte",
            format!("priority=gte.{}&id=in.(1,2,3)", rng.gen_range(5)),
        ),
        4 => (
            "lt",
            format!("priority=lt.{}&id=in.(1,2,3)", 1 + rng.gen_range(5)),
        ),
        5 => (
            "lte",
            format!("priority=lte.{}&id=in.(1,2,3)", 1 + rng.gen_range(5)),
        ),
        6 => (
            "like",
            format!("title=like.*{}*&id=in.(1,2,3)", token(rng, 4)),
        ),
        7 => (
            "ilike",
            format!("title=ilike.*{}*&id=in.(1,2,3)", token(rng, 4)),
        ),
        8 => ("in", format!("id=in.({})", fixture_ids(rng))),
        9 => {
            let value = ["null", "true", "false"][rng.gen_range(3) as usize];
            ("is", format!("done=is.{value}&id=in.(1,2,3)"))
        }
        10 => ("not", format!("done=not.eq.{bool_txt}&id=in.(1,2,3)")),
        _ => ("eq", format!("title=eq.{}&id=in.(1,2,3)", token(rng, 6))),
    }
}

fn forged_jwt(rng: &mut Rng) -> String {
    let alg = ["none", "None", "NONE", "HS256"][rng.gen_range(4) as usize];
    let role = ["anon", "authenticated", "service_role", "postgres"][rng.gen_range(4) as usize];
    let header = serde_json::json!({"alg": alg, "typ": "JWT"});
    let payload = serde_json::json!({"role": role, "sub": token(rng, 16)});
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&header).expect("forged jwt header is json"));
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&payload).expect("forged jwt payload is json"));
    if alg.eq_ignore_ascii_case("none") {
        format!("{header}.{payload}.")
    } else {
        let mut sig = [0u8; 16];
        rng.fill(&mut sig);
        let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig);
        format!("{header}.{payload}.{sig}")
    }
}

fn generate(rng: &mut Rng) -> Vec<Case> {
    let mut cases = Vec::with_capacity(GENERATED_CASE_COUNT);
    cases.extend(rest_cases(rng));
    cases.extend(auth_cases(rng));
    debug_assert_eq!(cases.len(), GENERATED_CASE_COUNT);
    cases
}

fn rest_cases(rng: &mut Rng) -> Vec<Case> {
    let mut cases = Vec::new();
    let order_col = ["id", "title", "done", "priority"][rng.gen_range(4) as usize];
    let direction = if rng.gen_bool() { "asc" } else { "desc" };
    let columns = select_list(rng);
    let ids = fixture_ids(rng);
    cases.push(http(
        rng,
        &[
            "rest:route:GET /rest/v1/{relation}",
            "rest:query-param:select",
            "rest:query-param:order",
        ],
        "GET",
        format!("/rest/v1/todos?select={columns}&order={order_col}.{direction}&id=in.({ids})"),
        Key::Anon,
        None,
        &[],
        &[],
    ));

    let mut kinds: Vec<u32> = (0..12).collect();
    shuffle(rng, &mut kinds);
    for kind in kinds.into_iter().take(3) {
        let (unit, query) = filter_query(rng, kind);
        let unit_id = format!("rest:filter-operator:{unit}");
        cases.push(http(
            rng,
            &[unit_id.as_str()],
            "GET",
            format!("/rest/v1/todos?{query}"),
            Key::Anon,
            None,
            &[],
            &[],
        ));
    }

    let unknown = token(rng, 4);
    cases.push(http(
        rng,
        &["rest:route:GET /rest/v1/{relation}"],
        "GET",
        format!("/rest/v1/todos?id=zz{unknown}.1"),
        Key::Anon,
        None,
        &[],
        &[],
    ));
    let limit = 1 + rng.gen_range(3);
    let offset = rng.gen_range(3);
    let ids = fixture_ids(rng);
    cases.push(http(
        rng,
        &[
            "rest:query-param:limit",
            "rest:query-param:offset",
            "rest:query-param:order",
        ],
        "GET",
        format!(
            "/rest/v1/todos?select=id,title&id=in.({ids})&order=id.asc&limit={limit}&offset={offset}"
        ),
        Key::Anon,
        None,
        &[],
        &[],
    ));
    let count = if rng.gen_bool() {
        "count=exact"
    } else {
        "count=planned"
    };
    let ids = fixture_ids(rng);
    cases.push(http(
        rng,
        &["rest:prefer:count=exact"],
        "GET",
        format!("/rest/v1/todos?select=id&id=in.({ids})"),
        Key::Anon,
        None,
        &[("Prefer", count)],
        &[],
    ));
    let row = 1 + rng.gen_range(3);
    cases.push(http(
        rng,
        &["rest:media-type:application/vnd.pgrst.object+json"],
        "GET",
        format!("/rest/v1/todos?id=eq.{row}"),
        Key::Anon,
        None,
        &[("Accept", "application/vnd.pgrst.object+json")],
        &[],
    ));
    let missing = token(rng, 8);
    cases.push(http(
        rng,
        &["rest:route:GET /rest/v1/{relation}"],
        "GET",
        format!("/rest/v1/mb{missing}"),
        Key::Anon,
        None,
        &[],
        &[],
    ));
    let title = format!("held-{}", token(rng, 12));
    let done = rng.gen_bool();
    let priority = i64::from(rng.gen_range(6));
    cases.push(http(
        rng,
        &["rest:route:POST /rest/v1/{relation}"],
        "POST",
        "/rest/v1/todos".into(),
        Key::Anon,
        Some(object(vec![
            ("title", toml_string(&title)),
            ("done", toml::Value::Boolean(done)),
            ("priority", toml::Value::Integer(priority)),
        ])),
        &[],
        &[],
    ));
    let a = i64::from(rng.gen_range(200)) - 100;
    let b = i64::from(rng.gen_range(200)) - 100;
    cases.push(http(
        rng,
        &["rest:route:POST /rest/v1/rpc/{function}"],
        "POST",
        "/rest/v1/rpc/add_numbers".into(),
        Key::Anon,
        Some(object(vec![
            ("a", toml::Value::Integer(a)),
            ("b", toml::Value::Integer(b)),
        ])),
        &[],
        &[],
    ));
    let a = rng.gen_range(50);
    let b = rng.gen_range(50);
    cases.push(http(
        rng,
        &["rest:route:GET /rest/v1/rpc/{function}"],
        "GET",
        format!("/rest/v1/rpc/add_numbers?a={a}&b={b}"),
        Key::Anon,
        None,
        &[],
        &[],
    ));
    let title = format!("held-{}", token(rng, 12));
    let done = rng.gen_bool();
    let priority = i64::from(rng.gen_range(6));
    cases.push(http(
        rng,
        &["rest:route:POST /rest/v1/{relation}"],
        "POST",
        "/rest/v1/todos".into(),
        Key::ServiceRole,
        Some(object(vec![
            ("title", toml_string(&title)),
            ("done", toml::Value::Boolean(done)),
            ("priority", toml::Value::Integer(priority)),
        ])),
        &[("Prefer", "return=representation")],
        &[],
    ));
    cases
}

fn auth_cases(rng: &mut Rng) -> Vec<Case> {
    let mut cases = Vec::new();
    let address = email(rng);
    let secret = password(rng);
    cases.push(http(
        rng,
        &["auth:route:POST /auth/v1/signup"],
        "POST",
        "/auth/v1/signup".into(),
        Key::Anon,
        Some(object(vec![
            ("email", toml_string(&address)),
            ("password", toml_string(&secret)),
        ])),
        &[],
        &[],
    ));
    let address = email(rng);
    let secret = weak_password(rng);
    cases.push(http(
        rng,
        &["auth:route:POST /auth/v1/signup"],
        "POST",
        "/auth/v1/signup".into(),
        Key::Anon,
        Some(object(vec![
            ("email", toml_string(&address)),
            ("password", toml_string(&secret)),
        ])),
        &[],
        &[],
    ));

    let address = email(rng);
    let secret = password(rng);
    let mut grant = http(
        rng,
        &[
            "auth:route:POST /auth/v1/signup",
            "auth:route:POST /auth/v1/token",
            "auth:grant-type:password",
        ],
        "POST",
        "/auth/v1/signup".into(),
        Key::Anon,
        Some(object(vec![
            ("email", toml_string(&address)),
            ("password", toml_string(&secret)),
        ])),
        &[],
        &[],
    );
    push_step(
        &mut grant,
        one_step(
            "POST",
            "/auth/v1/token?grant_type=password".into(),
            Key::Anon,
            Some(object(vec![
                ("email", toml_string(&address)),
                ("password", toml_string(&secret)),
            ])),
            &[],
            &[],
        ),
    );
    cases.push(grant);

    let address = email(rng);
    let secret = password(rng);
    let wrong = format!("{secret}x");
    let mut bad = http(
        rng,
        &["auth:route:POST /auth/v1/token", "auth:grant-type:password"],
        "POST",
        "/auth/v1/signup".into(),
        Key::Anon,
        Some(object(vec![
            ("email", toml_string(&address)),
            ("password", toml_string(&secret)),
        ])),
        &[],
        &[],
    );
    push_step(
        &mut bad,
        one_step(
            "POST",
            "/auth/v1/token?grant_type=password".into(),
            Key::Anon,
            Some(object(vec![
                ("email", toml_string(&address)),
                ("password", toml_string(&wrong)),
            ])),
            &[],
            &[],
        ),
    );
    cases.push(bad);

    let address = email(rng);
    let secret = password(rng);
    let mut refresh = http(
        rng,
        &[
            "auth:route:POST /auth/v1/token",
            "auth:grant-type:refresh_token",
        ],
        "POST",
        "/auth/v1/signup".into(),
        Key::Anon,
        Some(object(vec![
            ("email", toml_string(&address)),
            ("password", toml_string(&secret)),
        ])),
        &[],
        &[("refresh", "/refresh_token")],
    );
    push_step(
        &mut refresh,
        one_step(
            "POST",
            "/auth/v1/token?grant_type=refresh_token".into(),
            Key::Anon,
            Some(object(vec![("refresh_token", toml_string("{{refresh}}"))])),
            &[],
            &[],
        ),
    );
    cases.push(refresh);

    let address = email(rng);
    let secret = password(rng);
    let mut user = http(
        rng,
        &["auth:route:GET /auth/v1/user"],
        "POST",
        "/auth/v1/signup".into(),
        Key::Anon,
        Some(object(vec![
            ("email", toml_string(&address)),
            ("password", toml_string(&secret)),
        ])),
        &[],
        &[("token", "/access_token")],
    );
    push_step(
        &mut user,
        one_step(
            "GET",
            "/auth/v1/user".into(),
            Key::Anon,
            None,
            &[("Authorization", "Bearer {{token}}")],
            &[],
        ),
    );
    cases.push(user);

    cases.push(http(
        rng,
        &["auth:route:GET /auth/v1/user"],
        "GET",
        "/auth/v1/user".into(),
        Key::Anon,
        None,
        &[],
        &[],
    ));
    let token = forged_jwt(rng);
    let bearer = format!("Bearer {token}");
    cases.push(http(
        rng,
        &["auth:route:GET /auth/v1/user"],
        "GET",
        "/auth/v1/user".into(),
        Key::Anon,
        None,
        &[("Authorization", &bearer)],
        &[],
    ));
    cases
}

/// Builds the published record from pass bits. `generated` and `stored` are source counts.
pub fn report_from_passes(
    generated: usize,
    stored: usize,
    passes: &[bool],
) -> Result<HiddenReport> {
    let total = u32::try_from(passes.len()).context("hidden suite case count does not fit")?;
    let passed = u32::try_from(passes.iter().filter(|pass| **pass).count())
        .context("hidden suite pass count does not fit")?;
    let generated =
        u32::try_from(generated).context("hidden suite generated count does not fit")?;
    let stored = u32::try_from(stored).context("hidden suite stored count does not fit")?;
    if total != generated + stored {
        bail!("hidden suite: internal count mismatch; {WITHHELD}");
    }
    Ok(HiddenReport {
        schema: 1,
        suite: "hidden".into(),
        total,
        passed,
        failed: total - passed,
        generated,
        stored,
    })
}

/// Pretty JSON of a [`HiddenReport`], including a trailing newline.
///
/// # Errors
///
/// Returns an error when the report cannot be serialized.
pub fn summary_json(report: &HiddenReport) -> Result<String> {
    let mut json = serde_json::to_string_pretty(report)?;
    json.push('\n');
    Ok(json)
}

/// Markdown table of pass/fail counts. It has no case ids, paths, or bodies.
pub fn summary_markdown(report: &HiddenReport) -> String {
    format!(
        "### Hidden judge\n\n\
         | | |\n\
         |---|---|\n\
         | Total | {} |\n\
         | Passed | {} |\n\
         | Failed | {} |\n\
         | Generated | {} |\n\
         | Stored | {} |\n\n\
         Case identities, requests, and response diffs are not published.\n",
        report.total, report.passed, report.failed, report.generated, report.stored
    )
}

fn run_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    format!(
        "{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    )
}

/// Sends `assembled` to both stacks. Transport and database errors are withheld.
///
/// # Errors
///
/// Returns [`WITHHELD`] when a case cannot be executed (the reference stack
/// or a database check failed). A Megabase transport error is a failed case.
pub fn execute(
    assembled: &Assembled,
    reference: &run::Target,
    megabase: &run::Target,
    keys: &run::Keys,
    databases: &db::Databases,
) -> Result<HiddenReport> {
    let stamp = run_stamp();
    let mut passes = Vec::with_capacity(assembled.cases.len());
    for case in &assembled.cases {
        match run::run_case(case, reference, megabase, keys, &stamp, databases) {
            Ok(outcome) => passes.push(outcome.pass),
            Err(_) => bail!("{WITHHELD}"),
        }
    }
    report_from_passes(assembled.generated, assembled.stored, &passes)
}

/// `MEGABASE_JUDGE_HIDDEN_ALLOW_OPEN=1` is required before plaintext is written.
pub fn open_permitted(flag: Option<&str>) -> bool {
    flag == Some("1")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(text: &str) -> HiddenSeed {
        HiddenSeed::parse(text).unwrap()
    }

    fn test_seed() -> HiddenSeed {
        seed("megabase-judge-hidden-test-seed-value!!")
    }

    fn other_seed() -> HiddenSeed {
        seed("megabase-judge-hidden-other-seed-value!")
    }

    fn paths(assembled: &Assembled) -> Vec<String> {
        assembled
            .cases
            .iter()
            .flat_map(|case| case.step.iter().map(|step| step.path.clone()))
            .collect()
    }

    #[test]
    fn seed_debug_is_redacted_and_short_seeds_fail() {
        let parsed = test_seed();
        assert_eq!(format!("{parsed:?}"), "HiddenSeed([redacted])");
        assert!(HiddenSeed::parse("short").is_err());
        assert!(HiddenSeed::parse("   ").is_err());
    }

    #[test]
    fn generation_is_deterministic_and_seed_dependent() {
        let first = assemble(&test_seed(), None).unwrap();
        let second = assemble(&test_seed(), None).unwrap();
        assert_eq!(first.generated, GENERATED_CASE_COUNT);
        assert_eq!(first.stored, 0);
        assert_eq!(first.cases.len(), GENERATED_CASE_COUNT);
        assert_eq!(paths(&first), paths(&second));
        let ids: Vec<_> = first.cases.iter().map(|case| case.id.clone()).collect();
        let again: Vec<_> = second.cases.iter().map(|case| case.id.clone()).collect();
        assert_eq!(ids, again);
        assert_ne!(
            paths(&first),
            paths(&assemble(&other_seed(), None).unwrap())
        );
    }

    #[test]
    fn generated_cases_stay_on_gateway_prefixes_and_omit_the_seed() {
        let secret = "megabase-judge-hidden-test-seed-value!!";
        let assembled = assemble(&seed(secret), None).unwrap();
        let mut ids = std::collections::BTreeSet::new();
        for case in &assembled.cases {
            assert!(ids.insert(case.id.clone()), "duplicate id");
            assert!(case.id.starts_with('h') && case.id.len() == 17);
            assert!(case.description.is_empty());
            assert!(!case.units.is_empty());
            assert!(!case.step.is_empty());
            for step in &case.step {
                assert!(
                    step.path.starts_with("/rest/v1/") || step.path.starts_with("/auth/v1/"),
                    "{}",
                    step.path
                );
                assert!(step.method == "GET" || step.method == "POST");
                assert!(step.body.is_none());
                assert!(!step.path.contains(secret));
                if let Some(json) = &step.json {
                    let rendered = json.to_string();
                    assert!(!rendered.contains(secret));
                    assert!(!rendered.contains("judge-password-1"));
                    if rendered.contains("@example.com") {
                        assert!(rendered.contains("{{run}}"));
                    }
                }
            }
        }
    }

    #[test]
    fn summary_omits_case_material() {
        let assembled = assemble(&test_seed(), None).unwrap();
        let report = report_from_passes(assembled.generated, 0, &[true, false]).unwrap_err();
        assert!(report.to_string().contains(WITHHELD) || report.to_string().contains("mismatch"));
        let passes = vec![true; assembled.generated];
        let mut passes = passes;
        passes[0] = false;
        let report = report_from_passes(assembled.generated, 0, &passes).unwrap();
        let json = summary_json(&report).unwrap();
        let markdown = summary_markdown(&report);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "failed",
                "generated",
                "passed",
                "schema",
                "stored",
                "suite",
                "total"
            ]
        );
        assert_eq!(report.failed, 1);
        assert_eq!(report.passed, GENERATED_CASE_COUNT as u32 - 1);
        for case in &assembled.cases {
            assert!(!json.contains(&case.id));
            assert!(!markdown.contains(&case.id));
            for step in &case.step {
                assert!(!json.contains(&step.path));
                assert!(!markdown.contains(&step.path));
                if let Some(body) = &step.json {
                    let rendered = body.to_string();
                    if rendered.len() > 12 {
                        assert!(!json.contains(&rendered));
                        assert!(!markdown.contains(&rendered));
                    }
                }
            }
        }
        assert!(markdown.contains("not published"));
    }

    #[test]
    fn seal_roundtrip_and_authentication() {
        let toml = r#"
[[case]]
id = "stored.one"
units = ["auth:route:GET /auth/v1/health"]
description = "held out on purpose"
[[case.step]]
method = "GET"
path = "/auth/v1/health"
"#;
        let blob = seal(&test_seed(), toml.as_bytes()).unwrap();
        assert!(!blob.contains("stored.one"));
        assert!(!blob.contains("/auth/v1/health"));
        let opened = open(&test_seed(), &blob).unwrap();
        assert_eq!(opened, toml.as_bytes());
        assert!(open(&other_seed(), &blob).is_err());

        let assembled = assemble(&test_seed(), Some(&blob)).unwrap();
        assert_eq!(assembled.stored, 1);
        assert_eq!(assembled.generated, GENERATED_CASE_COUNT);
        assert!(assembled.cases.iter().any(|case| case.id == "stored.one"));
        let report = report_from_passes(
            assembled.generated,
            assembled.stored,
            &vec![true; assembled.cases.len()],
        )
        .unwrap();
        let published = summary_json(&report).unwrap() + &summary_markdown(&report);
        assert!(!published.contains("stored.one"));
        assert!(!published.contains("/auth/v1/health"));
        assert!(!published.contains("held out on purpose"));

        let mut raw = base64::engine::general_purpose::STANDARD
            .decode(&blob)
            .unwrap();
        let mid = raw.len() / 2;
        raw[mid] ^= 0xff;
        let tampered = base64::engine::general_purpose::STANDARD.encode(&raw);
        let err = open(&test_seed(), &tampered).unwrap_err().to_string();
        assert!(err.contains("authentication") || err.contains("supported"));
        assert!(!err.contains("stored.one"));

        raw[mid] ^= 0xff;
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        let tampered = base64::engine::general_purpose::STANDARD.encode(&raw);
        assert!(open(&test_seed(), &tampered).is_err());
    }

    #[test]
    fn invalid_blob_errors_do_not_echo_plaintext() {
        let marker = "this-plaintext-must-not-leak";
        let blob = seal(&test_seed(), marker.as_bytes()).unwrap();
        let err = match assemble(&test_seed(), Some(&blob)) {
            Err(err) => err.to_string(),
            Ok(_) => panic!("invalid plaintext must not assemble"),
        };
        assert!(!err.contains(marker));

        let duplicate = r#"
[[case]]
id = "secret-case-name"
units = ["auth:route:GET /auth/v1/health"]
[[case.step]]
method = "GET"
path = "/auth/v1/health"

[[case]]
id = "secret-case-name"
units = ["auth:route:GET /auth/v1/health"]
[[case.step]]
method = "GET"
path = "/auth/v1/settings"
"#;
        let blob = seal(&test_seed(), duplicate.as_bytes()).unwrap();
        let err = match assemble(&test_seed(), Some(&blob)) {
            Err(err) => err.to_string(),
            Ok(_) => panic!("duplicate case ids must not assemble"),
        };
        assert!(!err.contains("secret-case-name"));
        assert!(!err.contains("/auth/v1/settings"));
        assert!(err.contains(WITHHELD));
    }

    #[test]
    fn open_permit_is_exact() {
        assert!(open_permitted(Some("1")));
        assert!(!open_permitted(Some("true")));
        assert!(!open_permitted(Some("")));
        assert!(!open_permitted(None));
    }

    #[test]
    fn hidden_workflow_publishes_counts_only() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.github/workflows/judge-hidden.yml");
        let text = std::fs::read_to_string(&path).expect("judge-hidden.yml");
        assert!(text.contains("megabase-judge hidden"));
        assert!(text.contains("environment:"));
        assert!(text.contains("judge-hidden"));
        assert!(text.contains("refs/heads/main"));
        assert!(text.contains("schedule:"));
        assert!(text.contains("MEGABASE_JUDGE_HIDDEN_SEED"));
        assert!(!text.contains("hidden-open"));
        assert!(!text.contains("hidden-seal"));
        assert!(!text.contains("megabase.log"));
        assert!(!text.contains("pull_request:"));
        assert!(!text.contains("echo \"$MEGABASE_JUDGE_HIDDEN"));
    }
}
