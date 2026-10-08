#!/usr/bin/env bash
# Provision the Apple code-signing secrets for the release workflow.
#
#   bash scripts/setup-apple-signing.sh
#
# Signing a Mac app for distribution OUTSIDE the App Store needs a "Developer ID
# Application" identity: a certificate issued by Apple PLUS the private key that
# produced its signing request. Both halves matter — a downloaded .cer alone has
# no private key and can never codesign, which is the single most common way this
# is set up wrong.
#
# Apple has no API for issuing this; the certificate itself must be created in a
# browser. So this script does every part that CAN be automated — generating the
# signing request, importing the issued certificate, exporting the identity,
# base64-encoding it, and uploading all six GitHub secrets — and stops to tell you
# exactly what to click for the one part that cannot.
#
# Requires: macOS, and `gh` authenticated (brew install gh && gh auth login).
set -euo pipefail

REPO="${REPO:-moomoi/moo}"
WORK="${TMPDIR:-/tmp}/moo-apple-signing"
CSR="$WORK/DeveloperID.certSigningRequest"
KEY="$WORK/DeveloperID.key"
P12="$WORK/DeveloperID.p12"

say()  { printf '\n\033[1m%s\033[0m\n' "$*"; }
step() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }
warn() { printf '\033[1;33m%s\033[0m\n' "$*"; }
die()  { printf '\033[1;31mERROR: %s\033[0m\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = "Darwin" ] || die "macOS only — codesigning identities live in the login keychain."
command -v gh >/dev/null 2>&1 || die "gh not found. brew install gh && gh auth login"
gh auth status >/dev/null 2>&1 || die "gh is not authenticated. Run: gh auth login"

mkdir -p "$WORK"
chmod 700 "$WORK"

say "Apple code-signing setup for $REPO"
cat <<'INTRO'
This provisions six repository secrets:

  APPLE_CERTIFICATE           base64 of the Developer ID .p12
  APPLE_CERTIFICATE_PASSWORD  its export password
  APPLE_SIGNING_IDENTITY      "Developer ID Application: Name (TEAMID)"
  APPLE_TEAM_ID               10-character Team ID
  APPLE_ID                    your Apple ID email
  APPLE_PASSWORD              an app-specific password (NOT your account password)

You need a paid Apple Developer Program membership. Without one, stop here: the
release workflow refuses to publish an unsigned build.

If another repo already uses this Developer ID, the values have to be entered again:
GitHub never shows a secret once it's saved.
INTRO

# ── 1. reuse an existing identity if there is one ──────────────────────────────
step "Looking for an existing Developer ID Application identity"
EXISTING="$(security find-identity -v -p codesigning 2>/dev/null | grep "Developer ID Application" || true)"
IDENTITY=""
if [ -n "$EXISTING" ]; then
  echo "$EXISTING"
  read -r -p $'\nUse an identity above instead of creating a new one? [Y/n] ' reuse
  if [ "${reuse:-Y}" != "n" ] && [ "${reuse:-Y}" != "N" ]; then
    IDENTITY="$(printf '%s' "$EXISTING" | head -1 | sed -E 's/.*"(.*)".*/\1/')"
    echo "using: $IDENTITY"
  fi
else
  echo "none found in the login keychain."
fi

# ── 2. create one via a CSR if needed ─────────────────────────────────────────
if [ -z "$IDENTITY" ]; then
  step "Generating a certificate signing request"
  read -r -p "Email for the request: " CSR_EMAIL
  read -r -p "Common name (your name or company): " CSR_NAME
  [ -n "$CSR_EMAIL" ] && [ -n "$CSR_NAME" ] || die "both fields are required"

  # 2048-bit RSA is what Apple's CA expects for this certificate type.
  openssl req -new -newkey rsa:2048 -nodes \
    -keyout "$KEY" -out "$CSR" \
    -subj "/emailAddress=$CSR_EMAIL/CN=$CSR_NAME/C=US" >/dev/null 2>&1
  chmod 600 "$KEY"
  echo "wrote $CSR"

  say "MANUAL STEP — Apple issues the certificate through the browser only"
  cat <<INSTRUCTIONS
  1. Open https://developer.apple.com/account/resources/certificates/add
  2. Choose "Developer ID Application", then Continue.
  3. Upload this file:
       $CSR
  4. Download the issued certificate (developerID_application.cer).
INSTRUCTIONS
  read -r -p $'\nPath to the downloaded .cer [~/Downloads/developerID_application.cer]: ' CER
  CER="${CER:-$HOME/Downloads/developerID_application.cer}"
  CER="${CER/#\~/$HOME}"
  [ -f "$CER" ] || die "no such file: $CER"

  step "Building the .p12 from the certificate + the key that requested it"
  # Pair Apple's certificate with OUR private key. This is the whole point: the
  # .cer on its own cannot sign anything.
  openssl x509 -inform DER -in "$CER" -out "$WORK/cert.pem" 2>/dev/null \
    || openssl x509 -inform PEM -in "$CER" -out "$WORK/cert.pem"

  P12_PASS="$(openssl rand -base64 24)"
  openssl pkcs12 -export -legacy \
    -inkey "$KEY" -in "$WORK/cert.pem" \
    -out "$P12" -passout pass:"$P12_PASS" >/dev/null 2>&1 \
  || openssl pkcs12 -export \
    -inkey "$KEY" -in "$WORK/cert.pem" \
    -out "$P12" -passout pass:"$P12_PASS"
  chmod 600 "$P12"

  IDENTITY="$(openssl x509 -in "$WORK/cert.pem" -noout -subject 2>/dev/null \
    | sed -E 's/.*CN ?= ?([^,\/]*).*/\1/' | sed 's/[[:space:]]*$//')"
  echo "identity: $IDENTITY"

  step "Installing it into your login keychain (so local builds can sign too)"
  security import "$P12" -k "$HOME/Library/Keychains/login.keychain-db" \
    -P "$P12_PASS" -T /usr/bin/codesign >/dev/null 2>&1 \
    || warn "keychain import skipped — CI will still work; local signing may not."
fi

# ── 3. export an existing identity ────────────────────────────────────────────
if [ ! -f "$P12" ]; then
  step "Exporting the identity as a .p12"
  P12_PASS="$(openssl rand -base64 24)"
  warn "Keychain will prompt for your LOGIN password to release the private key."
  security export -t identities -f pkcs12 -o "$P12" -P "$P12_PASS" \
    || die "export failed. If the identity has no private key it cannot sign — recreate it with this script."
  chmod 600 "$P12"
fi

# ── 4. validate before uploading ──────────────────────────────────────────────
step "Verifying the .p12 opens with its password"
openssl pkcs12 -in "$P12" -passin pass:"$P12_PASS" -nokeys -legacy >/dev/null 2>&1 \
  || openssl pkcs12 -in "$P12" -passin pass:"$P12_PASS" -nokeys >/dev/null 2>&1 \
  || die "the .p12 is not readable with its own password — refusing to upload a secret that cannot work."
# A .p12 with no private key imports but never signs; catch it here rather than mid-release.
openssl pkcs12 -in "$P12" -passin pass:"$P12_PASS" -nocerts -nodes -legacy 2>/dev/null | grep -q "PRIVATE KEY" \
  || openssl pkcs12 -in "$P12" -passin pass:"$P12_PASS" -nocerts -nodes 2>/dev/null | grep -q "PRIVATE KEY" \
  || die "this .p12 contains no private key — it can never codesign. Recreate the identity."
echo "ok — certificate and private key both present"

# ── 5. remaining values ───────────────────────────────────────────────────────
step "Remaining values"
TEAM_ID="$(printf '%s' "$IDENTITY" | sed -nE 's/.*\(([A-Z0-9]{10})\).*/\1/p')"
if [ -z "$TEAM_ID" ]; then
  read -r -p "Team ID (10 chars, from developer.apple.com > Membership): " TEAM_ID
else
  echo "Team ID from the identity: $TEAM_ID"
fi
read -r -p "Apple ID email: " APPLE_ID_VALUE
say "App-specific password"
cat <<'APPPW'
Notarization will not accept your normal account password. Create one at:
  https://appleid.apple.com  >  Sign-In and Security  >  App-Specific Passwords
It looks like: abcd-efgh-ijkl-mnop
APPPW
read -r -s -p "App-specific password: " APPLE_PW; echo

# ── 6. upload ─────────────────────────────────────────────────────────────────
step "Uploading secrets to $REPO"
set_secret() { printf '%s' "$2" | gh secret set "$1" --repo "$REPO" >/dev/null && echo "  set $1"; }
set_secret APPLE_CERTIFICATE          "$(base64 -i "$P12")"
set_secret APPLE_CERTIFICATE_PASSWORD "$P12_PASS"
set_secret APPLE_SIGNING_IDENTITY     "$IDENTITY"
set_secret APPLE_TEAM_ID              "$TEAM_ID"
set_secret APPLE_ID                   "$APPLE_ID_VALUE"
set_secret APPLE_PASSWORD             "$APPLE_PW"

step "Cleaning up"
# The .p12 and private key are now in GitHub secrets and your keychain; leaving
# plaintext copies in a temp dir is a liability.
rm -rf "$WORK"
echo "removed $WORK"

say "Done."
cat <<'DONE'
The next release build will sign and notarize Moo.app and the DMG. To confirm
afterwards, on the downloaded DMG:

  bash scripts/verify-release.sh ~/Downloads/Moo-macos-universal.dmg
  spctl -a -vv -t exec /Volumes/Moo/Moo.app
  # expect: source=Notarized Developer ID
DONE
