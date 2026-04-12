#!/usr/bin/env python3
import subprocess
import sys
import threading
import time
import os

# ANSI color codes for terminal distinction
COLORS = {
    'SERVER': '\033[95m',   # Magenta
    'CLIENT1': '\033[96m',  # Cyan
    'CLIENT2': '\033[92m',  # Green
    'RESET': '\033[0m',     # Reset
}

def stream_output(process, prefix, color_key):
    """Reads lines from the subprocess and prints them with a colored prefix."""
    color = COLORS.get(color_key, COLORS['RESET'])
    
    # Read line by line as they come
    for line in iter(process.stdout.readline, ''):
        if not line:
            break
        # Print with prefix and color. Using rstrip() to remove the newline of the line, 
        # then print takes care of adding exactly one newline.
        print(f"{color}[{prefix}] {line.rstrip()}{COLORS['RESET']}", flush=True)

def main():
    print("🚀 Starting Dark Rift Cluster...")
    
    # Configure logging for robustness and deep inspection.
    # We set default to info, but enable debug for our crates and the networking protocol (lightyear).
    # We quiet down extremely noisy graphics/window crates to focus on mechanics and networking.
    env = os.environ.copy()
    env["RUST_LOG"] = "info,dark_rift_server=debug,dark_rift_client=debug,dark_rift_shared=debug,lightyear_netcode=info,aeronet=warn,wgpu=error,naga=error,winit=error,bevy_app=warn"
    
    # Helper to spawn a tracked process
    def spawn_process(name, cargo_pkg):
        return subprocess.Popen(
            ["cargo", "run", "-p", cargo_pkg],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,  # Merge stderr into stdout
            text=True,
            bufsize=1,                 # Line-buffered
            env=env
        )

    processes = []
    
    try:
        # 1. Start Server
        print("🛠️  Building and launching Server...")
        server_p = spawn_process("SERVER", "dark-rift-server")
        processes.append(("SERVER", server_p))
        threading.Thread(target=stream_output, args=(server_p, "SERVER", "SERVER"), daemon=True).start()
        
        # Give server a moment to compile and bind the websocket port
        time.sleep(3)
        
        # 2. Start Client 1
        print("🎮 Building and launching Client 1...")
        client1_p = spawn_process("CLIENT-1", "dark-rift-client")
        processes.append(("CLIENT-1", client1_p))
        threading.Thread(target=stream_output, args=(client1_p, "CLIENT-1", "CLIENT1"), daemon=True).start()
        
        time.sleep(1)
        
        # 3. Start Client 2
        print("🎮 Building and launching Client 2...")
        client2_p = spawn_process("CLIENT-2", "dark-rift-client")
        processes.append(("CLIENT-2", client2_p))
        threading.Thread(target=stream_output, args=(client2_p, "CLIENT-2", "CLIENT2"), daemon=True).start()

        # Wait indefinitely until interrupted or processes exit naturally
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
