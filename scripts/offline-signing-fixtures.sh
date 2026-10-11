#!/usr/bin/env bash
# Makes a throwaway self-signed "developer" certificate (.p12, password testpw) and a wildcard
# development profile listing FAKE-UDID-0001, for core/src/identity.rs's ignored offline_sign
# test. Nothing here is a real Apple certificate; the signed app won't install anywhere.
#   scripts/offline-signing-fixtures.sh <dir> <some.ipa>
#   APPARITION_OFFLINE=<dir> cargo test --release --lib offline_sign -- --ignored --nocapture
set -euo pipefail
dir=$1
mkdir -p "$dir"
cd "$dir"
openssl req -x509 -newkey rsa:2048 -nodes -keyout key.pem -out cert.pem -days 30 \
    -subj "/CN=Apple Worldwide Developer Relations Test: Fake (FAKE123)/OU=TEAM123456/O=Fake Team/C=US" 2>/dev/null
openssl pkcs12 -export -inkey key.pem -in cert.pem -out id.p12 -passout pass:testpw
openssl x509 -in cert.pem -outform der -out cert.der
expires=$(date -u -v+20d +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -d '+20 days' +%Y-%m-%dT%H:%M:%SZ)
cat > profile.plist <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Name</key><string>Fake Wildcard</string>
<key>UUID</key><string>11111111-2222-3333-4444-555555555555</string>
<key>TeamIdentifier</key><array><string>TEAM123456</string></array>
<key>TeamName</key><string>Fake Team</string>
<key>Platform</key><array><string>iOS</string><string>xrOS</string></array>
<key>ExpirationDate</key><date>$expires</date>
<key>ProvisionedDevices</key><array><string>FAKE-UDID-0001</string></array>
<key>DeveloperCertificates</key><array><data>$(base64 < cert.der | tr -d '\n')</data></array>
<key>Entitlements</key><dict>
<key>application-identifier</key><string>TEAM123456.*</string>
<key>com.apple.developer.team-identifier</key><string>TEAM123456</string>
<key>get-task-allow</key><true/>
<key>keychain-access-groups</key><array><string>TEAM123456.*</string></array>
<key>com.apple.developer.associated-domains</key><string>*</string>
<key>com.apple.developer.kernel.increased-memory-limit</key><true/>
</dict></dict></plist>
EOF
openssl cms -sign -nodetach -binary -in profile.plist -signer cert.pem -inkey key.pem -outform der -out fake.mobileprovision
cp "$2" app.ipa
echo "fixtures in $dir"
