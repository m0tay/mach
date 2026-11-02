cask "mach" do
  version "0.15.0"
  sha256 "e64d1f429a954fc5114c1dc3530ad5660c53e5d39bfc150130b55ca45e66aa09"

  url "https://github.com/m0tay/mach/releases/download/v0.15.0/mach-0.15.0-aarch64-apple-darwin.tar.gz"
  name "Mach"
  desc "Supersonic Linux virtualization & container engine for Apple Silicon"
  homepage "https://github.com/m0tay/mach"

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
    run "{{HOMEBREW_PREFIX}}/bin/machctl", args: ["init", "--non-interactive"]
  end

  uninstall early_script: {
              executable: "/bin/sh",
              args:       ["-c", "/usr/bin/pkill -9 -f 'mach menu' 2>/dev/null || true; /usr/bin/pkill -9 -f 'machctl menu' 2>/dev/null || true"],
            },
            script: {
              executable: "#{HOMEBREW_PREFIX}/bin/machctl",
              args:       ["uninstall", "--keep-data", "--force"],
              must_succeed: false,
            }

  zap trash: [
    "~/.mach",
    "~/Mach",
  ]

  caveats <<~EOS
    Mach is installed for your Apple Silicon Mac.

    Quickstart:
      • Open Linux shell:         mach
      • Run Linux commands:       mach uname -a
      • Start Docker:              machctl docker start
      • Enable Kubernetes:         machctl k8s enable
      • Activity dashboard:       machctl top
      • Open files in Finder:     open ~/Mach
      • macOS Menu bar:           mach menu
  EOS
end
