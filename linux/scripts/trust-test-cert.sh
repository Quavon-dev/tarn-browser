#!/bin/sh
docker exec tarn-p2 sh -c 'echo 127.0.0.1 trusted.test >> /etc/hosts && cp /tmp/cert.pem /etc/pki/ca-trust/source/anchors/tarn-test.pem && update-ca-trust'
