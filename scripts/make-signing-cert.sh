#!/bin/sh
# Create a self-signed code-signing certificate (no Apple subscription needed) for release builds.
# macOS ties privacy grants (screen/mic/camera) to the app's bundle ID plus its signing certificate,
# so signing every release with the SAME certificate keeps those grants across updates.
# Run this once, store the output as GitHub secrets, and keep the .p12 safe: a new cert means
# re-granting permissions once.
set -eu
NAME="${1:-SimpleRecord Release Signing}"
OUT="${2:-simple-record-signing}"
PASS=$(openssl rand -base64 18)
cat > "$OUT.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
CNF
openssl req -x509 -newkey rsa:2048 -nodes -days 36500 -keyout "$OUT.key" -out "$OUT.crt" -config "$OUT.cnf"
# Legacy PBE algorithms so macOS `security import` can read the file.
openssl pkcs12 -export -inkey "$OUT.key" -in "$OUT.crt" -out "$OUT.p12" -passout "pass:$PASS" \
  -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1
rm -f "$OUT.key" "$OUT.crt" "$OUT.cnf"
echo "Created $OUT.p12. Add these GitHub repository secrets (Settings > Secrets and variables > Actions):"
echo "  SIGNING_CERT_P12_BASE64 = $(base64 < "$OUT.p12" | tr -d '\n')"
echo "  SIGNING_CERT_PASSWORD   = $PASS"
echo "  SIGNING_IDENTITY        = $NAME"
echo "Then delete $OUT.p12 or store it in a password manager. Never commit it."
