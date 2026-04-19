#!/usr/bin/env python3
"""
Stress test cluster: server + 2 regular clients + 24 network bots.
Network bots randomly pick between tortuga, agresivo, and suicida profiles.
"""
import os
import subprocess
import sys
import threading
import time
import random

COLORS = {
    "SERVER": "\033[95m",
    "CLIENT-1": "\033[96m",
    "CLIENT-2": "\033[92m",
    "BOT": "\033[93m",
    "RESET": "\033[0m",
}

def stream_output(process, prefix, color_key, suppress_output=False):
    color = COLORS.get(color_key, COLORS["RESET"])
    for line in iter(process.stdout.readline, ""):
        if not line:
            break
        if not suppress_output:
            print(f"{color}[{prefix}] {line.rstrip()}{COLORS['RESET']}", flush=True)

    code = process.wait()
    if not suppress_output:
        if code > 0:
            print(f"{color}[{prefix}] *** exited with error {code} ***{COLORS['RESET']}", flush=True)
        elif code < 0:
            print(f"{color}[{prefix}] (stopped by signal {-code}){COLORS['RESET']}", flush=True)

def main():
    print("🚀 Starting Dark Rift Network Stress Cluster (2 Clients + 24 Network Bots)...")

    env = os.environ.copy()
    env.setdefault("RUST_BACKTRACE", "1")
    
    # Add target/release/deps and rust toolchain libs to LD_LIBRARY_PATH
    deps_path = os.path.abspath("target/release/deps")
    try:
        sysroot = subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()
        rustc_info = subprocess.check_output(["rustc", "-vV"], text=True)
        host_target = next(line.split("host: ")[1].strip() for line in rustc_info.splitlines() if line.startswith("host: "))
        rustlib_path = f"{sysroot}/lib/rustlib/{host_target}/lib"
        sys_lib_path = f"{sysroot}/lib"
        paths_to_add = f"{deps_path}:{rustlib_path}:{sys_lib_path}"
    except Exception as e:
        print(f"⚠️ Failed to get rust sysroot: {e}")
        paths_to_add = deps_path

    if "LD_LIBRARY_PATH" in env:
        env["LD_LIBRARY_PATH"] = f"{paths_to_add}:{env['LD_LIBRARY_PATH']}"
    else:
        env["LD_LIBRARY_PATH"] = paths_to_add

    # FIX: Bevy Asset Server expects CARGO_MANIFEST_DIR to locate `assets/` when run directly
    env["CARGO_MANIFEST_DIR"] = os.path.abspath(".")

    if "RUST_LOG" not in env:
        env["RUST_LOG"] = (
            "warn,"
            "dark_rift_server=info,dark_rift_client=info,dark_rift_shared=info,"
            "lightyear_netcode=warn,aeronet=warn,"
            "wgpu=error,naga=error,winit=error,bevy_app=warn"
        )

    use_release = os.environ.get("DARK_RIFT_DEBUG", "") != "1"

    def build_packages():
        print("🛠️  Compiling release binaries (Server & Client)...")
        subprocess.run(["cargo", "build", "--release"], env=env, check=True)

    def spawn_binary(bin_name: str, extra_args: list = None):
        cmd = [f"./target/release/{bin_name}"]
        if extra_args:
            cmd.extend(extra_args)
        return subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
            env=env,
        )

    processes = []

    try:
        build_packages()

        print("🛠️  Launching Server...")
        server_p = spawn_binary("dark-rift-server")
        processes.append(("SERVER", server_p))
        threading.Thread(target=stream_output, args=(server_p, "SERVER", "SERVER"), daemon=True).start()

        time.sleep(3) # Wait for server to bind

        print("🎮 Launching Client 1...")
        client1_p = spawn_binary("dark-rift-client")
        processes.append(("CLIENT-1", client1_p))
        threading.Thread(target=stream_output, args=(client1_p, "CLIENT-1", "CLIENT-1"), daemon=True).start()

        time.sleep(1)

        print("🎮 Launching Client 2...")
        client2_p = spawn_binary("dark-rift-client")
        processes.append(("CLIENT-2", client2_p))
        threading.Thread(target=stream_output, args=(client2_p, "CLIENT-2", "CLIENT-2"), daemon=True).start()

        time.sleep(2)

        bot_profiles = ["agresivo", "suicida", "tortuga"]
        num_bots = 36
        
        print(f"🤖 Launching {num_bots} Network Bots concurrently...")
        for i in range(num_bots):
            profile = random.choice(bot_profiles)
            args = ["--bot", "--profile", f"assets/bots/{profile}.ron"]
            bot_p = spawn_binary("dark-rift-client", args)
            name = f"BOT-{i+1}"
            processes.append((name, bot_p))
            # We suppress the output of the 120 bots so the terminal doesn't become pure spam
            threading.Thread(target=stream_output, args=(bot_p, name, "BOT", False), daemon=True).start()
            # 0.01 seconds stagger = 1.2 seconds total to launch 120 bots
            time.sleep(0.01)

        print(f"\n✅ Stress Cluster fully booted ({num_bots} bots + 2 clients)! Press Ctrl+C to shutdown.\n")
        
        # Wait on server indefinitely
        server_p.wait()

    except KeyboardInterrupt:
        print("\n🛑 Shutdown sequence initiated (Ctrl+C caught)... killing processes.")
    finally:
        for name, p in processes:
            if p.poll() is None:
                p.terminate()
                try:
                    p.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    p.kill()
        print("✨ Cluster shutdown complete.")
        sys.exit(0)

if __name__ == "__main__":
    main()
