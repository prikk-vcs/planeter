#!/bin/sh
# Regenerates the THROWAWAY test certificates of planeter-client's tests (nothing real trusts them; the keys are public in the
# repository). `good` is valid for ten years from the day it is made; `exp` is expired by fixed dates, so it stays expired.
# Needs openssl. Run from this directory.
set -e
rm -f *.pem *.key *.csr *.srl *.ext
for ca in ca ca2; do
  openssl req -x509 -newkey rsa:2048 -nodes -keyout $ca.key -out $ca.pem -days 3650 -subj "/CN=planeter-client test $ca" \
    -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null
done
printf "subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n" > san.ext
printf "subjectAltName=DNS:other.example\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n" > wrong.ext
leaf() { # name ca ext [dates]
  openssl req -newkey rsa:2048 -nodes -keyout $1.key -out $1.csr -subj "/CN=localhost" 2>/dev/null
  openssl x509 -req -in $1.csr -CA $2.pem -CAkey $2.key -CAcreateserial -out $1.pem -extfile $3 $4 2>/dev/null
}
leaf good ca san.ext "-days 3650"
leaf wrong ca wrong.ext "-days 3650"
leaf exp ca san.ext "-not_before 20200101000000Z -not_after 20200102000000Z"
leaf other ca2 san.ext "-days 3650"
rm -f *.csr *.srl *.ext ca.key ca2.key
ls
