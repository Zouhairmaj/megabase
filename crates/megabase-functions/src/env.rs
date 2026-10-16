// Ported from supabase/edge-runtime ext/env/lib.rs and ext/env/env.js (MIT), pin v1.77.4.

//! `Deno.env` of a user worker, as edge-runtime's `env` extension serves it.
//!
//! An edge-runtime user worker never touches the process environment.
//! `Deno.env` is `SUPABASE_ENV` (`ext/env/env.js`), backed by the `EnvVars`
//! map the worker was created with (`ext/env/lib.rs`): reads see that map,
//! `set` throws `NotSupported`, and `delete` does nothing. A plain Deno
//! process reads and writes its real environment, so the bootstrap replaces
//! `Deno.env` with [`SOURCE`] before the function's module graph is loaded.
//! Spec: `specs/functions/env.md`.

/// Environment variables the worker process gets from Megabase but an
/// edge-runtime worker never sees, removed from the snapshot.
pub const HIDDEN: &[&str] = &["DENO_SERVE_ADDRESS", crate::worker::ENTRYPOINT_ENV];

/// JavaScript that snapshots the environment and installs `Deno.env`.
///
/// It runs after the bootstrap has read its own settings and before the
/// function is imported. The snapshot is the process environment without
/// [`HIDDEN`], plus `SB_EXECUTION_ID` (a fresh v4 UUID per worker, like the
/// worker key edge-runtime's pool inserts in `crates/base/src/runtime/mod.rs`).
// megabase:unit functions:op:op_set_env
// megabase:unit functions:op:op_env
// megabase:unit functions:op:op_get_env
// megabase:unit functions:op:op_delete_env
pub const SOURCE: &str = r#"
// Deno.env of an edge-runtime user worker (crates/megabase-functions/src/env.rs).
{
  const envVars = new Map(Object.entries(Deno.env.toObject()));
  for (const key of ["DENO_SERVE_ADDRESS", "MEGABASE_FUNCTION_ENTRYPOINT"]) envVars.delete(key);
  envVars.set("SB_EXECUTION_ID", crypto.randomUUID());

  // `format!("{:?}", key)` for the ASCII escapes Rust's `str` Debug uses.
  const debugString = (value) => {
    let out = "\"";
    for (const char of value) {
      const code = char.codePointAt(0);
      if (char === "\0") out += "\\0";
      else if (char === "\t") out += "\\t";
      else if (char === "\r") out += "\\r";
      else if (char === "\n") out += "\\n";
      else if (char === "\\") out += "\\\\";
      else if (char === "\"") out += "\\\"";
      else if (code < 0x20 || (code >= 0x7f && code <= 0x9f)) out += `\\u{${code.toString(16)}}`;
      else out += char;
    }
    return out + "\"";
  };

  // op_get_env: the permission check always passes (user workers get
  // `allow_env`), then the key is validated and looked up.
  const getEnv = (key) => {
    if (typeof key !== "string") throw new TypeError("Expected string");
    if (key === "") throw new TypeError("Key is an empty string.");
    if (key.includes("=") || key.includes("\0")) {
      throw new TypeError(`Key contains invalid characters: ${debugString(key)}`);
    }
    return envVars.get(key);
  };

  const SUPABASE_ENV = {
    get: getEnv,
    // op_env: a fresh copy of the whole map.
    toObject() {
      return Object.fromEntries(envVars);
    },
    // op_set_env: `Err(not_supported())`.
    set(_key, _value) {
      throw new Deno.errors.NotSupported("The operation is not supported");
    },
    has(key) {
      return getEnv(key) !== undefined;
    },
    // op_delete_env: a no-op.
    delete(_key) {},
  };

  Object.defineProperty(Deno, "env", {
    value: SUPABASE_ENV,
    enumerable: true,
    writable: false,
    configurable: true,
  });
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_hides_every_megabase_variable() {
        for key in HIDDEN {
            assert!(
                SOURCE.contains(&format!("\"{key}\"")),
                "{key} is not removed from the snapshot"
            );
        }
        assert!(SOURCE.contains("envVars.set(\"SB_EXECUTION_ID\", crypto.randomUUID())"));
    }

    #[test]
    fn set_throws_not_supported_and_delete_is_a_no_op() {
        assert!(SOURCE
            .contains("throw new Deno.errors.NotSupported(\"The operation is not supported\")"));
        assert!(SOURCE.contains("delete(_key) {},"));
    }

    #[test]
    fn get_validates_the_key_like_op_get_env() {
        assert!(SOURCE.contains("throw new TypeError(\"Key is an empty string.\")"));
        assert!(SOURCE.contains("Key contains invalid characters: ${debugString(key)}"));
        assert!(SOURCE.contains("return getEnv(key) !== undefined;"));
    }

    #[test]
    fn env_is_installed_read_only_on_the_deno_namespace() {
        assert!(SOURCE.contains("Object.defineProperty(Deno, \"env\""));
        assert!(SOURCE.contains("writable: false"));
    }
}
