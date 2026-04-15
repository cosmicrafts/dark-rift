#!/usr/bin/env python3
"""
Dark Rift Deployment & Orchestration Tool.

Usage Modes:
1. Default (Local Dev):
   ./run_cluster.py
   Builds and runs 1 local server and 2 local clients connecting to it.

2. Cloud Deploy (Server):
   ./run_cluster.py --deploy <IP>
   Compiles the server binary locally (release mode), rsyncs it and assets to the VPS,
   sets up a systemd service, and restarts it on the target machine.

3. Stress Test:
   ./run_cluster.py --stress 120 --server <IP:5005>
   Spawns 120 local clients pointing to the cloud server IP to test its limits.

4. Web Deploy (WASM):
   ./run_cluster.py --deploy <IP> --web
   Compiles the client to WASM, bundles it, and uploads it directly to the Nginx web directory.
"""
import argparse
import os
import subprocess
import sys
import threading
import time
import shutil

COLORS = {
    "SERVER": "\033[95m",
    "CLIENT1": "\033[96m",
    "CLIENT2": "\033[92m",
    "STRESS": "\033[93m",
    "RESET": "\033[0m",
}

def stream_output(process, prefix, color_key):
    color = COLORS.get(color_key, COLORS["RESET"])
    for line in iter(process.stdout.readline, ""):
        if not line:
            break
        print(f"{color}[{prefix}] {line.rstrip()}{COLORS['RESET']}", flush=True)
    process.wait()

def run_cmd(cmd, desc):
    print(f"⚙️  {desc}...")
    result = subprocess.run(cmd, shell=True, text=True)
    if result.returncode != 0:
        print(f"❌ Failed: {desc}")
        sys.exit(1)
    print(f"✅ Success: {desc}")

def build_and_deploy_server(ip):
    print(f"🚀 Deploying Server to {ip}...")
    run_cmd("rustup target add x86_64-unknown-linux-musl", "Ensuring musl target for static cross-compilation")
    
    # We use CC_x86_64_unknown_linux_musl=x86_64-linux-musl-gcc for Mac/Linux cross-compilation
    build_cmd = 'CC_x86_64_unknown_linux_musl="x86_64-linux-musl-gcc" CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="x86_64-linux-musl-gcc" cargo build -p dark-rift-server --release --target x86_64-unknown-linux-musl'
    run_cmd(build_cmd, "Building Static Server Binary (Release)")
    
    remote_dir = "/home/bizkit/dark-rift-prod"
    run_cmd(f"ssh bizkit@{ip} 'mkdir -p {remote_dir}/assets'", "Preparing Remote Directory")
    
    print("📦 Uploading Binary and Assets via rsync...")
    run_cmd(f"scp target/x86_64-unknown-linux-musl/release/dark-rift-server bizkit@{ip}:{remote_dir}/", "SCP Binary")
    run_cmd(f"scp -r crates/client/assets bizkit@{ip}:{remote_dir}/", "SCP Assets")
    
    service_file = """[Unit]
Description=Dark Rift Server
After=network.target

[Service]
Type=simple
User=bizkit
WorkingDirectory=/home/bizkit/dark-rift-prod
ExecStart=/home/bizkit/dark-rift-prod/dark-rift-server
Restart=always
RestartSec=5
Environment=RUST_LOG=info,dark_rift_server=info,lightyear_netcode=warn

[Install]
WantedBy=default.target"""
    
    print("🔧 Configuring Systemd Service...")
    run_cmd(f"echo '{service_file}' | ssh bizkit@{ip} 'sudo -n tee /etc/systemd/system/darkrift-server.service > /dev/null'", "Writing Service File")
    run_cmd(f"ssh bizkit@{ip} 'sudo -n systemctl daemon-reload && sudo -n systemctl enable darkrift-server && sudo -n systemctl restart darkrift-server'", "Restarting Service")
    print(f"🎉 Deployment Complete! The server is now running on {ip}:5005")

def build_and_deploy_web(ip):
    print(f"🌐 Building WebAssembly Client for {ip}...")
    run_cmd("rustup target add wasm32-unknown-unknown", "Ensuring wasm32 target")
    # Using local cargo if wasm-bindgen is not installed globally it might fail, best effort:
    if shutil.which("wasm-bindgen") is None:
        run_cmd("cargo install wasm-bindgen-cli", "Installing wasm-bindgen-cli")
        
    run_cmd("cargo build -p dark-rift-client --target wasm32-unknown-unknown --release", "Building WASM")
    
    os.makedirs("web-dist", exist_ok=True)
    run_cmd("wasm-bindgen --out-dir ./web-dist/out --target web ./target/wasm32-unknown-unknown/release/dark_rift_client.wasm", "Binding WASM")
    run_cmd("cp -r crates/client/assets ./web-dist/", "Copying Assets")
    
    html_template = """<!DOCTYPE html><html>
<head><meta charset="utf-8"/><title>Dark Rift</title><style>body { margin: 0; background: #000; overflow: hidden; } canvas { width: 100vw; height: 100vh; }</style></head>
<body><script type="module">import init from './out/dark_rift_client.js'; init();</script></body></html>"""
    with open("web-dist/index.html", "w") as f:
        f.write(html_template)
        
    print("📦 Uploading Web Version via rsync to Nginx...")
    remote_web_dir = "/var/www/worldofunreal.com/darkrift"
    run_cmd(f"ssh bizkit@{ip} 'sudo -n mkdir -p {remote_web_dir} && sudo -n chown -R bizkit:bizkit {remote_web_dir}'", "Preparing Web Directory")
    run_cmd(f"rsync -avz --progress web-dist/ bizkit@{ip}:{remote_web_dir}/", "Rsync Web Files")
    print(f"🎉 Web Deployment Complete! Available at the configured domain.")

def run_stress_test(clients, server):
    print(f"🔥 Starting Stress Test: {clients} clients attacking {server}...")
    env = os.environ.copy()
    env["RUST_LOG"] = "error" # Quiet down for stress testing
    
    run_cmd("cargo build -p dark-rift-client --release", "Building Release Client for Stress Test")
    
    processes = []
    try:
        for i in range(clients):
            p = subprocess.Popen(
                ["cargo", "run", "--release", "-p", "dark-rift-client", "--", "--server", server],
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1, env=env
            )
            processes.append((f"C{i+1}", p))
            # Just silent them or minimal logging to not choke the terminal
            
        print(f"✅ Spawned {clients} clients. Press Ctrl+C to stop.")
        for name, p in processes:
            p.wait()
    except KeyboardInterrupt:
        print("\n🛑 Stress test interrupted. Killing all clients...")
        for name, p in processes:
            p.kill()

def main():
    parser = argparse.ArgumentParser(description="Dark Rift Deployment & Orchestration")
    parser.add_argument("--deploy", metavar="IP", help="Deploy the server to the given IP via SSH/Rsync")
    parser.add_argument("--web", action="store_true", help="Also build WASM and deploy Web Client to Nginx (requires --deploy)")
    parser.add_argument("--stress", type=int, metavar="N", help="Run N clients locally for a stress test")
    parser.add_argument("--server", default="127.0.0.1:5005", help="Server address for clients (default: 127.0.0.1:5005)")
    args = parser.parse_args()

    if args.deploy:
        build_and_deploy_server(args.deploy)
        if args.web:
            build_and_deploy_web(args.deploy)
        sys.exit(0)
        
    if args.stress:
        run_stress_test(args.stress, args.server)
        sys.exit(0)

    # DEFAULT MODE: Run Cluster Locally
    print("🚀 Starting Local Dark Rift Cluster...")
    env = os.environ.copy()
    env.setdefault("RUST_BACKTRACE", "1")
    if "RUST_LOG" not in env:
        env["RUST_LOG"] = "warn,dark_rift_server=info,dark_rift_client=info,dark_rift_shared=info"

    use_release = os.environ.get("DARK_RIFT_DEBUG", "") != "1"
    
    def spawn_process(pkg, extra_args=None):
        cmd = ["cargo", "run", "-p", pkg]
        if use_release:
            cmd.insert(2, "--release")
        if extra_args:
            cmd.extend(["--"] + extra_args)
        return subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1, env=env)

    if use_release:
        print("(Release build. For fast compile while hacking: DARK_RIFT_DEBUG=1 ./run_cluster.py)\n")
        
    processes = []
    try:
        print("🛠️  Building and launching Server...")
        server_p = spawn_process("dark-rift-server")
        processes.append(("SERVER", server_p))
        threading.Thread(target=stream_output, args=(server_p, "SERVER", "SERVER"), daemon=True).start()
        time.sleep(3)

        print("🎮 Building and launching Client 1...")
        client1_p = spawn_process("dark-rift-client", ["--server", args.server])
        processes.append(("CLIENT-1", client1_p))
        threading.Thread(target=stream_output, args=(client1_p, "CLIENT-1", "CLIENT1"), daemon=True).start()
        time.sleep(1)

        print("🎮 Building and launching Client 2...")
        client2_p = spawn_process("dark-rift-client", ["--server", args.server])
        processes.append(("CLIENT-2", client2_p))
        threading.Thread(target=stream_output, args=(client2_p, "CLIENT-2", "CLIENT2"), daemon=True).start()

        print("\n✅ Cluster fully booted! Press Ctrl+C to shutdown all instances securely.\n")
        server_p.wait()
        client1_p.wait()
        client2_p.wait()
    except KeyboardInterrupt:
        print("\n🛑 Shutdown sequence initiated...")
    finally:
        for name, p in processes:
            if p.poll() is None:
                p.terminate()
                try: p.wait(timeout=2)
                except: p.kill()
        sys.exit(0)

if __name__ == "__main__":
    main()
