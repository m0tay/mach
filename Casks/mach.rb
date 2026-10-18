cask "mach" do
  version "0.6.0"
  sha256 "1e6614ddbc2cc0297e33884b40715eae1f679666fe0f23c3f5042497e4161176"

  url "file:///Users/douglaslobo/Developer/mach/dist/mach-0.6.0-aarch64-apple-darwin.tar.gz"
  name "Mach"
  desc "Supersonic Linux virtualization & container engine for Apple Silicon"
  homepage "https://github.com/douglaslobo/mach"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  binary "mach"
  binary "machctl"
  manpage "man/man1/mach.1"
  manpage "man/man1/machctl.1"
  bash_completion "completions/mach.bash"
  zsh_completion "completions/_mach"
  zsh_completion "completions/_machctl"
  fish_completion "completions/mach.fish"

  postflight_steps do
    run "/usr/bin/xattr", args: ["-cr", "{{staged_path}}"]
    run "/usr/bin/codesign", args: ["--force", "--entitlements", "{{staged_path}}/entitlements.plist", "--sign", "-", "{{staged_path}}/mach"]
    run "{{HOMEBREW_PREFIX}}/bin/mach", args: ["init", "--non-interactive"]
  end

  uninstall early_script: {
              executable: "/bin/sh",
              args:       ["-c", "/usr/bin/pkill -9 -f 'mach menu' 2>/dev/null || true; /usr/bin/pkill -9 -f 'machctl menu' 2>/dev/null || true"],
            },
            script: {
              executable: "#{HOMEBREW_PREFIX}/bin/machctl",
              args:       ["uninstall", "--all", "--force"],
              must_succeed: false,
            }

  zap trash: [
    "~/.mach",
    "~/Mach",
  ]

  caveats <<~EOS
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
