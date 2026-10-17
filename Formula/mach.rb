class Mach < Formula
  desc "Supersonic Linux virtualization & container engine for Apple Silicon"
  homepage "https://github.com/douglaslobo/mach"
  url "file:///Users/douglaslobo/Developer/norb/dist/mach-0.1.0-aarch64-apple-darwin.tar.gz"
  sha256 "1d4ef8715309d3c26ae803eae168d8e9eb9c8369dc04665cecb690d7c1cb98bd"
  license "MIT"

  depends_on arch: :arm64
  depends_on :macos

  def install
    bin.install "mach"
    bin.install_symlink "mach" => "machctl"

    # Install assets
    (pkgshare/"assets").install Dir["assets/*"]

    # Install man pages
    man1.install "man/man1/mach.1"
    man1.install "man/man1/machctl.1"

    # Codesign with hypervisor and virtualization entitlements
    system "codesign", "--force", "--entitlements", "entitlements.plist", "--sign", "-", bin/"mach"

    generate_completions_from_executable(bin/"mach", "completion")
  end

  def post_install_steps
    # Initialize Mach environment, configs, keys, contexts safely without overwriting user data
    system bin/"mach", "init", "--non-interactive"
  end

  def caveats
    <<~EOS
      ⚡️ mach is initialized and ready on your Apple Silicon Mac!

      Quickstart:
        • Open Linux shell:         mach
        • Run Linux commands:       mach uname -a
        • Docker containers:        docker ps
        • Kubernetes cluster:       kubectl get nodes
        • Activity dashboard:       machctl top
        • Open files in Finder:     open ~/Mach
        • macOS Menu bar:           mach menu
    EOS
  end

  test do
    system "#{bin}/mach", "--version"
    system "#{bin}/machctl", "doctor"
  end
end
