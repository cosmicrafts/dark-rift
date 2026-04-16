#!/usr/bin/env python3
"""
Cluster: server + 2 clients.

- Default: `cargo run --release` (dev sin optimizar suele sentirse “injugable”).
  Desarrollo rápido: `DARK_RIFT_DEBUG=1 ./run_cluster.py` (build debug).
- Logs: por defecto `info` en nuestros crates; `RUST_LOG` en el entorno lo sustituye entero.
- Ctrl+C / SIGINT en Unix da código negativo (p. ej. -2): no es panic.
"""
import os
import subprocess
import sys
import threading
import time

COLORS = {
    "SERVER": "\033[95m",
    "CLIENT1": "\033[96m",
    "CLIENT2": "\033[92m",
    "RESET": "\033[0m",
}


def _describe_exit(code: int) -> str:
    if code == 0:
        return ""
    if code < 0:
        sig = -code
        return f"stopped by signal {sig} (e.g. Ctrl+C → SIGINT is 2 → often -2); not a Rust panic"
    return f"exited with error code {code}"


def stream_output(process, prefix, color_key):
    color = COLORS.get(color_key, COLORS["RESET"])
    for line in iter(process.stdout.readline, ""):
        if not line:
            break
        print(f"{color}[{prefix}] {line.rstrip()}{COLORS['RESET']}", flush=True)

    code = process.wait()
    msg = _describe_exit(code)
    if code > 0:
        print(
            f"{color}[{prefix}] *** {msg} (see above; RUST_BACKTRACE=1 on panic) ***{COLORS['RESET']}",
            flush=True,
        )
    elif code < 0:
        print(f"{color}[{prefix}] ({msg}){COLORS['RESET']}", flush=True)


def main():
    print("🚀 Starting Dark Rift Cluster...")

    env = os.environ.copy()
    env.setdefault("RUST_BACKTRACE", "1")

    if "RUST_LOG" not in env:
        env["RUST_LOG"] = (
            "warn,"
            "dark_rift_server=info,dark_rift_client=info,dark_rift_shared=info,"
            "lightyear_netcode=warn,aeronet=warn,"
            "wgpu=error,naga=error,winit=error,bevy_app=warn"
        )

    use_release = os.environ.get("DARK_RIFT_DEBUG", "") != "1"

    def cargo_run_cmd(pkg: str) -> list:
        cmd = ["cargo", "run", "-p", pkg]
        if use_release:
            cmd.insert(2, "--release")
        return cmd

    def spawn_process(name, cargo_pkg):
        return subprocess.Popen(
            cargo_run_cmd(cargo_pkg),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
            env=env,
        )

    if use_release:
        print("(Release build. For fast compile while hacking: DARK_RIFT_DEBUG=1 ./run_cluster.py)\n")
    processes = []

    try:
        print("🛠️  Building and launching Server...")
        server_p = spawn_process("SERVER", "dark-rift-server")
        processes.append(("SERVER", server_p))
        threading.Thread(
            target=stream_output, args=(server_p, "SERVER", "SERVER"), daemon=True
        ).start()

        time.sleep(3)

        print("🎮 Building and launching Client 1...")
        client1_p = spawn_process("CLIENT-1", "dark-rift-client")
        processes.append(("CLIENT-1", client1_p))
        threading.Thread(
            target=stream_output, args=(client1_p, "CLIENT-1", "CLIENT1"), daemon=True
        ).start()

        time.sleep(1)

        print("🎮 Building and launching Client 2...")
        client2_p = spawn_process("CLIENT-2", "dark-rift-client")
        processes.append(("CLIENT-2", client2_p))
        threading.Thread(
            target=stream_output, args=(client2_p, "CLIENT-2", "CLIENT2"), daemon=True
        ).start()

        print("\n✅ Cluster fully booted! Press Ctrl+C to shutdown all instances securely.\n")
        server_p.wait()
        client1_p.wait()
        client2_p.wait()

    except KeyboardInterrupt:
        print("\n🛑 Shutdown sequence initiated (Ctrl+C caught)... killing processes.")
    finally:
        for name, p in processes:
            if p.poll() is None:
                print(f"Terminating {name}...")
                p.terminate()
                try:
                    p.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    print(f"Force killing {name}...")
                    p.kill()
        print("✨ Cluster shutdown complete.")
        sys.exit(0)


if __name__ == "__main__":
    main()
