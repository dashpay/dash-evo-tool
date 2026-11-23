#!/usr/bin/env bash
set -euo pipefail
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  xvfb openbox x11-utils xdotool scrot mesa-vulkan-drivers libvulkan1 \
  libegl1 libgl1 libxkbcommon-x11-0 libxcursor1 libxrandr2 libxi6 libx11-xcb1 \
  at-spi2-core python3-pyatspi python3-pil python3-numpy python3-mnemonic \
  fonts-dejavu-core dbus-x11
# Keep the controller and permitted executables outside Claude's write scope.
sudo mkdir -p /opt/release-qa/visual
sudo cp scripts/ci/release-qa/*.py /opt/release-qa/
sudo cp .claude/skills/visual-diff/scripts/*.py /opt/release-qa/visual/
sudo chmod 755 /opt/release-qa/ui.py
sudo ln -sf /opt/release-qa/ui.py /usr/local/bin/qa-ui
sudo npm install --global @anthropic-ai/claude-code@2.1.331
python3 .claude/skills/visual-diff/scripts/selftest.py
