// Ported from supabase/edge-runtime ext/os/os.js, ext/runtime/lib.rs (MIT), pin v1.77.4.

//! Runtime facts an edge-runtime user worker reports that a plain Deno
//! process reports differently.
//!
//! `Deno.consoleSize()` is `{ columns: 80, rows: 24 }` in a user worker
//! (`ext/os/os.js:46`) whatever the host terminal is, and
//! `Deno.memoryUsage()` reports `rss: 0` (`ext/runtime/lib.rs:339`). Spec:
//! `specs/functions/runtime.md`.

/// JavaScript that installs the fixed console size and the memory usage
/// without the process RSS. It runs before the function is imported.
// megabase:unit functions:op:op_runtime_memory_usage
pub const SOURCE: &str = r#"
// Deno.consoleSize of an edge-runtime user worker (crates/megabase-functions/src/runtime.rs).
Object.defineProperty(Deno, "consoleSize", {
  value: () => ({ columns: 80, rows: 24 }),
  enumerable: true,
  writable: true,
  configurable: true,
});
// op_runtime_memory_usage: rss is hardcoded to 0 "for security".
const nativeMemoryUsage = Deno.memoryUsage;
Object.defineProperty(Deno, "memoryUsage", {
  value: () => {
    const { heapTotal, heapUsed, external } = nativeMemoryUsage.call(Deno);
    return { rss: 0, heapTotal, heapUsed, external };
  },
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

    #[test]
    fn memory_usage_hides_rss_and_keeps_the_heap_fields() {
        assert!(SOURCE.contains("Object.defineProperty(Deno, \"memoryUsage\""));
        assert!(SOURCE.contains("return { rss: 0, heapTotal, heapUsed, external };"));
    }
}
