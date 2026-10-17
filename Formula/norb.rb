class Norb < Formula
  desc "Lightweight, zero-overhead macOS virtualization and container engine in Rust"
  homepage "https://github.com/douglaslobo/norb"
  url "file:///Users/douglaslobo/Developer/norb", using: :git, branch: "main"
  version "0.1.0"
  license "MIT"
  head "https://github.com/douglaslobo/norb.git", branch: "main"

  depends_on :macos
  depends_on arch: :arm64

  def install
    system "cargo", "build", "--release", "--bin", "norb"
    bin.install "target/release/norb"
    bin.install_symlink "norb" => "norbctl"

    # Codesign with hypervisor and virtualization entitlements
    system "codesign", "--force", "--entitlements", "entitlements.plist", "--sign", "-", bin/"norb"

    # Shell autocompletions for bash, zsh, and fish
    (bash_completion/"norb").write Utils.safe_popen_read(bin/"norb", "completion", "bash")
    (zsh_completion/"_norb").write Utils.safe_popen_read(bin/"norb", "completion", "zsh")
    (fish_completion/"norb.fish").write Utils.safe_popen_read(bin/"norb", "completion", "fish")
  end

  def post_install
    ohai "Initializing Norb environment and provisioning Debian & Kubernetes..."
    # Run onboarding setup (creates Debian machine, k8s cluster node, and good-to-have integrations)
    system bin/"norb", "setup", "--welcome"
  rescue => e
    opoo "Automatic initialization deferred: #{e.message}"
    puts "You can complete environment setup at any time by simply running: norb setup"
  end

  def caveats
    <<~EOS
      🎉 Norb is installed and ready on your Apple Silicon Mac!

      Quickstart:
        • Drop into your Debian Linux shell:  norb
        • Run commands right from your Mac:  norb uname -a
        • Use Docker with zero overhead:     docker ps
        • Check your Kubernetes cluster:     kubectl get nodes
        • Browse Linux files in Finder:      open ~/Norb
        • Live resource activity monitor:    norb top
        • Apple menu bar status item:        norb menu &

      For a guided onboarding tour, run:
        norb setup
    EOS
  end

  test do
    system "#{bin}/norb", "--version"
    system "#{bin}/norbctl", "doctor"
  end
end
