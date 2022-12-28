_default:
  @just --list

install-models:
  scripts/install_models.sh

# Outdated, don't use
install-pytorch-mac:
  #!/bin/bash
  python3 -mvenv torch-install
  cd torch-install
  source bin/activate
  pip3 install -r requirements.txt

