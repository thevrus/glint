# version and sha256 are rewritten by .github/workflows/release.yml on every release.
cask "glint" do
  version "0.1.0"
  sha256 "85fd43e43f2910f359b77e39823dd1c7caf64b3204a61d8a0f87f3df78dd5579"

  url "https://github.com/thevrus/glint/releases/download/v#{version}/Glint_#{version}_universal.dmg"
  name "Glint"
  desc "Menu bar control for Logitech Litra lights"
  homepage "https://github.com/thevrus/glint"

  auto_updates true
  depends_on macos: :sonoma

  app "Glint.app"

  # The app is not notarized yet, so Gatekeeper would refuse to open the quarantined copy.
  postflight_steps do
    run "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "{{appdir}}/Glint.app"]
  end

  zap trash: "~/Library/Application Support/dev.glint.desktop"
end
