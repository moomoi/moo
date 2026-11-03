#!/usr/bin/env bash
# Provision the Microsoft Store (Partner Center) submission secrets for release-promoted.yml.
#
#   bash scripts/setup-windows-store.sh                      # this repo, Moo's product ID
#   REPO=duneyou/dune-ide PRODUCT_ID=9N7LFRB14H0C bash scripts/setup-windows-store.sh
#
# One Partner Center API login (a Microsoft Entra app) serves every app in the account, so run
# this once per repo with the same answers and that repo's Store ID.
#
# Partner Center has no API for creating the login, so do this in a browser first:
#   1. partner.microsoft.com -> Account settings -> User management -> Microsoft Entra applications
#      -> Add/Create Microsoft Entra application, role: Manager.
#   2. Open it -> Add new key. Copy the key (the client secret) now: it's shown once.
#   3. Note the Tenant ID and Client ID on that page, and the Seller ID under
#      Account settings -> Legal info -> Developer.
#
# Requires: gh, authenticated (gh auth login).
set -euo pipefail

REPO="${REPO:-moomoi/moo}"
PRODUCT_ID="${PRODUCT_ID:-9NQNCW0R1LFQ}"

die() { printf '\033[1;31mERROR: %s\033[0m\n' "$*" >&2; exit 1; }
command -v gh >/dev/null 2>&1 || die "gh not found. brew install gh && gh auth login"
gh auth status >/dev/null 2>&1 || die "gh is not authenticated. Run: gh auth login"

printf '\033[1mMicrosoft Store submission secrets for %s (Store ID %s)\033[0m\n\n' "$REPO" "$PRODUCT_ID"

ask() { # name prompt hidden?
  local v
  if [ "${3:-}" = hidden ]; then read -rsp "$2: " v; echo; else read -rp "$2: " v; fi
  [ -n "$v" ] || die "$2 is required"
  printf '%s' "$v"
}
TENANT_ID=$(ask TENANT_ID "Tenant ID")
CLIENT_ID=$(ask CLIENT_ID "Client ID")
CLIENT_SECRET=$(ask CLIENT_SECRET "Client secret (hidden)" hidden)
SELLER_ID=$(ask SELLER_ID "Seller ID")

printf '%s' "$TENANT_ID"     | gh secret set PARTNER_CENTER_TENANT_ID     --repo "$REPO"
printf '%s' "$CLIENT_ID"     | gh secret set PARTNER_CENTER_CLIENT_ID     --repo "$REPO"
printf '%s' "$CLIENT_SECRET" | gh secret set PARTNER_CENTER_CLIENT_SECRET --repo "$REPO"
printf '%s' "$SELLER_ID"     | gh secret set PARTNER_CENTER_SELLER_ID     --repo "$REPO"
gh variable set WINDOWS_STORE_PRODUCT_ID --body "$PRODUCT_ID" --repo "$REPO"

printf '\n\033[1;32mDone.\033[0m The next promoted release submits its MSIX to the Store.\n'
printf 'To resubmit an existing release: gh workflow run release-promoted.yml --repo %s -f tag=vX.Y.Z\n' "$REPO"
