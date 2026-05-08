#!/usr/bin/env python3
"""
Cluster: server + 2 clients.

- Default: `cargo run --release` (dev sin optimizar suele sentirse “injugable”).
  Desarrollo rápido: `DARK_RIFT_DEBUG=1 ./run_cluster.py` (build debug).
- Logs: por defecto `info` en nuestros crates; `RUST_LOG` en el entorno lo sustituye entero.
- Ctrl+C / SIGINT en Unix da código negativo (p. ej. -2): no es panic.
"""
import os
import argparse
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path
import re

COLORS = {
    "SERVER": "\033[95m",
    "CLIENT1": "\033[96m",
    "CLIENT2": "\033[92m",
    "RESET": "\033[0m",
}

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent
NETWORK_RS = PROJECT_ROOT / "crates" / "client" / "src" / "network" / "network.rs"
PATCH_BACKUP = SCRIPT_DIR / ".run_cluster.network.rs.backup"

DEFAULT_SERVER_ADDR = "127.0.0.1:25565"


def _describe_exit(code: int) -> str:
    if code == 0:
        return ""
    if code < 0:
        sig = -code
        return f"stopped by signal {sig} (e.g. Ctrl+C → SIGINT is 2 → often -2); not a Rust panic"
    return f"exited with error code {code}"


def _binary_path(pkg: str, use_release: bool) -> Path:
    profile = "release" if use_release else "debug"
    return PROJECT_ROOT / "target" / profile / pkg


def _patch_native_netcode_socket(content: str, server_addr: str) -> str:
    pattern = r'(?m)^(pub const NATIVE_NETCODE_SOCKET: &str = ")[^"]+(";)'
    patched, count = re.subn(pattern, rf'\g<1>{server_addr}\g<2>', content, count=1)
    if count != 1:
        raise RuntimeError("Could not patch NATIVE_NETCODE_SOCKET in network.rs")
    return patched


def _restore_if_backup_exists() -> bool:
    if not PATCH_BACKUP.exists():
        return False
    original = PATCH_BACKUP.read_text(encoding="utf-8")
    NETWORK_RS.write_text(original, encoding="utf-8")
    PATCH_BACKUP.unlink(missing_ok=True)
    return True


def _apply_temporary_patch(server_addr: str) -> bool:
    current = NETWORK_RS.read_text(encoding="utf-8")
    patched = _patch_native_netcode_socket(current, server_addr)
    if patched == current:
        return False
    PATCH_BACKUP.write_text(current, encoding="utf-8")
    NETWORK_RS.write_text(patched, encoding="utf-8")
    return True


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
    parser = argparse.ArgumentParser(
        description="Run Dark Rift local cluster (server + 2 clients) with safe temporary endpoint patch."
    )
    parser.add_argument(
        "--server",
        default=DEFAULT_SERVER_ADDR,
        help="Native client server socket to patch temporarily (default: 127.0.0.1:25565)",
    )
    mode_group = parser.add_mutually_exclusive_group()
    mode_group.add_argument(
        "--release",
        action="store_true",
        help="Force release build/run (max runtime performance; slower compile).",
    )
    mode_group.add_argument(
        "--debug",
        action="store_true",
        help="Force debug build/run (faster compile; lower runtime performance).",
    )
    parser.add_argument(
        "--restore-only",
        action="store_true",
        help="Restore network.rs from backup created by a previous interrupted run, then exit.",
    )
    args = parser.parse_args()

    os.chdir(PROJECT_ROOT)

    restored_on_start = _restore_if_backup_exists()
    if restored_on_start:
        print("🧯 Found stale temporary patch backup from previous run; restored network.rs first.")
    if args.restore_only:
        print("✅ Restore-only mode complete.")
        return

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

    if args.release:
        use_release = True
    elif args.debug:
        use_release = False
    else:
        use_release = os.environ.get("DARK_RIFT_DEBUG", "") != "1"

    profile = "release" if use_release else "debug"
    server_bin = _binary_path("dark-rift-server", use_release)
    client_bin = _binary_path("dark-rift-client", use_release)

    def build_once():
        build_cmd = ["cargo", "build"]
        if use_release:
            build_cmd.append("--release")
        build_cmd += ["-p", "dark-rift-server", "-p", "dark-rift-client"]
        subprocess.run(build_cmd, env=env, check=True)

    def spawn_process(name, cmd):
        return subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
            env=env,
        )

    if use_release:
        print("(Release build: max runtime performance, slower compile.)\n")
    else:
        print("(Debug build: faster compile, lower runtime performance.)\n")
    processes = []
    patch_applied = False
    handled_signal = {"num": None}

    def _signal_handler(signum, _frame):
        handled_signal["num"] = signum
        raise KeyboardInterrupt()

    signal.signal(signal.SIGINT, _signal_handler)
    signal.signal(signal.SIGTERM, _signal_handler)

    try:
        patch_applied = _apply_temporary_patch(args.server)
        if patch_applied:
            print(f"🔧 Patched native endpoint in network.rs -> {args.server} (temporary).")
        else:
            print(f"ℹ️ Native endpoint already set to {args.server}; no patch needed.")

        print(f"🛠️  Building binaries once ({profile})...")
        build_once()

        print("🛠️  Launching Server...")
        server_p = spawn_process("SERVER", [str(server_bin)])
        processes.append(("SERVER", server_p))
        threading.Thread(
            target=stream_output, args=(server_p, "SERVER", "SERVER"), daemon=True
        ).start()

        time.sleep(3)

        print("🎮 Launching Client 1...")
        client1_p = spawn_process("CLIENT-1", [str(client_bin)])
        processes.append(("CLIENT-1", client1_p))
        threading.Thread(
            target=stream_output, args=(client1_p, "CLIENT-1", "CLIENT1"), daemon=True
        ).start()

        time.sleep(1)

        print("🎮 Launching Client 2...")
        client2_p = spawn_process("CLIENT-2", [str(client_bin)])
        processes.append(("CLIENT-2", client2_p))
        threading.Thread(
            target=stream_output, args=(client2_p, "CLIENT-2", "CLIENT2"), daemon=True
        ).start()

        print("\n✅ Cluster fully booted! Press Ctrl+C to shutdown all instances securely.\n")
        server_p.wait()
        client1_p.wait()
        client2_p.wait()

    except KeyboardInterrupt:
        if handled_signal["num"] is not None:
            print(f"\n🛑 Shutdown sequence initiated (signal {handled_signal['num']} caught)... killing processes.")
        else:
            print("\n🛑 Shutdown sequence initiated (Ctrl+C caught)... killing processes.")
    except subprocess.CalledProcessError as e:
        print(f"\n❌ Build failed: {e}")
        raise
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
        if patch_applied:
            try:
                restored = _restore_if_backup_exists()
                if restored:
                    print("♻️ Restored network.rs to original endpoint.")
                else:
                    print("⚠️ No backup found during restore; verify network.rs manually.")
            except Exception as restore_err:
                print(f"❌ Failed to restore network.rs automatically: {restore_err}")
        print("✨ Cluster shutdown complete.")


if __name__ == "__main__":
    main()
