// Ported from supabase/edge-runtime ext/os/os.js (MIT), pin v1.77.4.

//! Runtime facts an edge-runtime user worker reports that a plain Deno
//! process reports differently.
//!
//! `Deno.consoleSize()` is `{ columns: 80, rows: 24 }` in a user worker
//! (`ext/os/os.js:46`) whatever the host terminal is. Spec:
//! `specs/functions/runtime.md`.

/// JavaScript that installs the fixed console size. It runs before the
/// function is imported.
pub const SOURCE: &str = r#"
// Deno.consoleSize of an edge-runtime user worker (crates/megabase-functions/src/runtime.rs).
Object.defineProperty(Deno, "consoleSize", {
  value: () => ({ columns: 80, rows: 24 }),
  enumerable: true,
  writable: true,
  configurable: true,
});
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_size_is_80_by_24() {
        assert!(SOURCE.contains("({ columns: 80, rows: 24 })"));
        assert!(SOURCE.contains("Object.defineProperty(Deno, \"consoleSize\""));
    }
}
