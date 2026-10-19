// Ported from supabase/edge-runtime ext/runtime/js/denoOverrides.js and ext/runtime/js/http.js (MIT), pin v1.77.4.

//! Worker bootstrap: makes every TCP server a function starts reach the
//! worker's own port, like an edge-runtime user worker.
//!
//! In edge-runtime a user worker has no real sockets. `Deno.listen` (any
//! address) and `Deno.serve` both receive the requests routed to the worker,
//! and `Deno.serveHttp` serves the accepted connections
//! (`ext/runtime/js/denoOverrides.js`, `ext/runtime/js/http.js`,
//! edge-runtime v1.77.4). So the legacy `serve()` of
//! `https://deno.land/std@0.168.0/http/server.ts`, which calls
//! `Deno.listen({ port: 8000 })` and `Deno.serveHttp(conn)`, works unchanged.
//!
//! A plain Deno 2 process binds the real port (two functions collide on
//! `:8000`) and no longer has `Deno.serveHttp`. Megabase therefore runs
//! [`module`] as the main module of `deno run` (Deno 2.1.4 has no
//! `--preload`), which imports the function. Deno 2.1.4 ignores
//! `DENO_SERVE_ADDRESS`, so the bootstrap wraps `Deno.serve` to bind the
//! worker's address whenever the function gives none. It starts one such
//! server the first time the function calls `Deno.listen` for TCP, hands each
//! request to that listener as a one-request connection, and answers
//! `Deno.serveHttp` for those connections. Other transports are untouched. See
//! `docs/decisions/0043-functions-listen-bootstrap.md`. Before importing the
//! function it also replaces `Deno.env` with the user-worker view in
//! [`crate::env`].

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// File name of the bootstrap inside its private directory.
pub const FILE_NAME: &str = "megabase-worker-bootstrap.js";

/// JavaScript that runs first in every worker, before the function's module
/// graph is evaluated.
pub const SOURCE: &str = r#"// Megabase edge function worker bootstrap (crates/megabase-functions/src/bootstrap.rs).
const nativeListen = Deno.listen;
const nativeServeHttp = Deno.serveHttp;
const nativeServe = Deno.serve;
const REQUEST = Symbol("megabase.request");
const listeners = [];
let server;

function unavailable(pending) {
  pending.reject(new Deno.errors.BadResource("Listener has been closed"));
}

class WorkerListener {
  constructor(addr) {
    this.addr = addr;
    this.closed = false;
    this.queue = [];
    this.waiters = [];
  }
  deliver(pending) {
    const conn = new WorkerConn(this.addr, pending);
    const waiter = this.waiters.shift();
    if (waiter) waiter.resolve(conn);
    else this.queue.push(conn);
  }
  accept() {
    if (this.queue.length > 0) return Promise.resolve(this.queue.shift());
    if (this.closed) {
      return Promise.reject(new Deno.errors.BadResource("Listener has been closed"));
    }
    return new Promise((resolve, reject) => this.waiters.push({ resolve, reject }));
  }
  close() {
    if (this.closed) return;
    this.closed = true;
    for (const waiter of this.waiters.splice(0)) {
      waiter.reject(new Deno.errors.BadResource("Listener has been closed"));
    }
    for (const conn of this.queue.splice(0)) unavailable(conn[REQUEST]);
  }
  ref() {}
  unref() {}
  async *[Symbol.asyncIterator]() {
    while (true) {
      try {
        yield await this.accept();
      } catch (error) {
        if (error instanceof Deno.errors.BadResource) return;
        throw error;
      }
    }
  }
  [Symbol.dispose]() {
    this.close();
  }
}

class WorkerConn {
  constructor(localAddr, pending) {
    this.localAddr = localAddr;
    this.remoteAddr = pending.info?.remoteAddr ??
      { transport: "tcp", hostname: "127.0.0.1", port: 0 };
    this[REQUEST] = pending;
  }
  close() {}
  closeWrite() {}
  ref() {}
  unref() {}
  [Symbol.dispose]() {}
}

class WorkerHttpConn {
  constructor(conn) {
    this.pending = conn[REQUEST];
    this.localAddr = conn.localAddr;
    this.remoteAddr = conn.remoteAddr;
  }
  nextRequest() {
    const pending = this.pending;
    this.pending = null;
    if (!pending) return Promise.resolve(null);
    return Promise.resolve({
      request: pending.request,
      respondWith(response) {
        return Promise.resolve(response).then((value) => {
          if (!(value instanceof Response)) {
            pending.resolve(new Response("Bad Gateway", { status: 502 }));
            throw new TypeError(
              "First argument to 'respondWith' must be a Response or a promise resolving to a Response",
            );
          }
          pending.resolve(value);
        }, (error) => {
          pending.reject(error);
          throw error;
        });
      },
    });
  }
  close() {
    const pending = this.pending;
    this.pending = null;
    if (pending) unavailable(pending);
  }
  async *[Symbol.asyncIterator]() {
    for (let event; (event = await this.nextRequest()) !== null;) yield event;
  }
  [Symbol.dispose]() {
    this.close();
  }
}

// Deno 2.1.4 ignores DENO_SERVE_ADDRESS and binds 0.0.0.0:8000 when
// `Deno.serve` gets no address, so force the worker address in that case.
const serveAddress = /^tcp:(.+):(\d+)$/.exec(Deno.env.get("DENO_SERVE_ADDRESS") ?? "");

function hasAddress(options) {
  return options.port !== undefined || options.hostname !== undefined ||
    options.path !== undefined || options.cid !== undefined ||
    options.transport !== undefined;
}

function withWorkerAddress(options) {
  if (!serveAddress || !options || typeof options !== "object" || hasAddress(options)) {
    return options;
  }
  return { ...options, hostname: serveAddress[1], port: Number(serveAddress[2]) };
}

Deno.serve = function serve(...args) {
  if (typeof args[0] === "function") {
    return nativeServe.call(Deno, withWorkerAddress({}), args[0]);
  }
  if (typeof args[1] === "function") {
    return nativeServe.call(Deno, withWorkerAddress(args[0]), args[1]);
  }
  return nativeServe.call(Deno, withWorkerAddress(args[0]));
};

function workerServer() {
  server ??= Deno.serve({ onListen() {} }, (request, info) =>
    new Promise((resolve, reject) => {
      const pending = { request, info, resolve, reject };
      const listener = listeners.find((candidate) => !candidate.closed);
      if (listener) listener.deliver(pending);
      else unavailable(pending);
    }));
  return server;
}

Deno.listen = function listen(options) {
  const transport = options?.transport ?? "tcp";
  if (transport !== "tcp") return nativeListen.call(Deno, options);
  const { hostname, port } = workerServer().addr;
  const listener = new WorkerListener({ transport: "tcp", hostname, port });
  listeners.push(listener);
  return listener;
};

Deno.serveHttp = function serveHttp(conn) {
  if (conn && conn[REQUEST]) return new WorkerHttpConn(conn);
  if (typeof nativeServeHttp === "function") return nativeServeHttp.call(Deno, conn);
  throw new TypeError("Deno.serveHttp is not a function");
};
"#;

/// The main module of every worker: [`SOURCE`], the read of the function's
/// entrypoint (path in `MEGABASE_FUNCTION_ENTRYPOINT`, removed from the
/// environment), the user-worker `Deno.env` ([`crate::env::SOURCE`]), the
/// runtime shims ([`crate::runtime::SOURCE`]), `Deno.systemMemoryInfo`
/// ([`crate::os::SOURCE`]), and finally the import of the entrypoint.
#[must_use]
pub fn module() -> String {
    format!(
        "{SOURCE}{ENTRYPOINT}{env}{runtime}{os}{IMPORT}",
        env = crate::env::SOURCE,
        runtime = crate::runtime::SOURCE,
        os = crate::os::SOURCE
    )
}

const ENTRYPOINT: &str = r#"
const entrypoint = Deno.env.get("MEGABASE_FUNCTION_ENTRYPOINT");
Deno.env.delete("MEGABASE_FUNCTION_ENTRYPOINT");
"#;

const IMPORT: &str = r#"
await import("file://" + encodeURI(entrypoint).replace(/[?#]/g, encodeURIComponent));
"#;

/// Write [`module`] to a new private directory under `parent` and return the
/// file's path. The directory name is unique per process and call, and it is
/// created (mode `0700` on Unix) only if it does not exist yet, so a
/// pre-planted path in a shared temporary directory is never followed.
///
/// # Errors
///
/// Any I/O error creating the directory or writing the file.
pub fn install(parent: &Path) -> std::io::Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    static INSTALLS: AtomicU64 = AtomicU64::new(0);
    let dir = parent.join(format!(
        "megabase-functions-{}-{nanos}-{}",
        std::process::id(),
        INSTALLS.fetch_add(1, Ordering::Relaxed)
    ));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(&dir)?;
    let path = dir.join(FILE_NAME);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(module().as_bytes())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_writes_the_source_into_a_fresh_private_directory() {
        let first = install(&std::env::temp_dir()).unwrap();
        let second = install(&std::env::temp_dir()).unwrap();
        assert_ne!(first.parent(), second.parent());
        assert_eq!(std::fs::read_to_string(&first).unwrap(), module());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(first.parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        for path in [first, second] {
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }

    #[test]
    fn module_wraps_serve_for_every_overload_and_loads_the_entrypoint_last() {
        let module = module();
        assert!(module.contains("DENO_SERVE_ADDRESS"));
        assert!(module.contains("Deno.serve = function serve(...args)"));
        // serve(handler), serve(options, handler) and serve({ handler })
        assert!(module.contains("typeof args[0] === \"function\""));
        assert!(module.contains("typeof args[1] === \"function\""));
        assert!(module.contains("nativeServe.call(Deno, withWorkerAddress(args[0]))"));
        let wrap = module.find("Deno.serve = function").unwrap();
        let import = module.find("await import(").unwrap();
        assert!(wrap < import);
    }

    #[test]
    fn module_installs_the_worker_env_after_reading_the_entrypoint_and_before_importing_it() {
        let module = module();
        // The worker `Deno.env.delete` is a no-op, so the real variable must
        // be removed while `Deno.env` is still Deno's own.
        let delete = module
            .find("Deno.env.delete(\"MEGABASE_FUNCTION_ENTRYPOINT\")")
            .unwrap();
        let env = module.find(crate::env::SOURCE).unwrap();
        let import = module.find("await import(").unwrap();
        assert!(delete < env && env < import);
        // `DENO_SERVE_ADDRESS` is read before the snapshot hides it.
        assert!(module.find("Deno.env.get(\"DENO_SERVE_ADDRESS\")").unwrap() < env);
    }

    #[test]
    fn module_installs_the_console_size_before_importing_the_function() {
        let module = module();
        let size = module.find(crate::runtime::SOURCE).unwrap();
        assert!(size < module.find("await import(").unwrap());
    }

    #[test]
    fn install_fails_when_the_parent_is_missing() {
        let parent = std::env::temp_dir().join("megabase-functions-no-such-parent/x");
        assert!(install(&parent).is_err());
    }
}
