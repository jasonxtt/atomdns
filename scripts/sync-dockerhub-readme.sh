#!/usr/bin/env bash
# Sets the Docker Hub overview (docker/README.md) and short description of the image repository.
#   scripts/sync-dockerhub-readme.sh
# Credentials: DOCKERHUB_USERNAME + DOCKERHUB_TOKEN (an access token), or, when unset, the login
# stored in ~/.docker/config.json. IMAGE=jasonxtt/atomdns replaces the repository.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${IMAGE:-jasonxtt/atomdns}"
SHORT="AtomDNS: Rust rewrite of MosDNS — DNS splitter with WebUI (amd64/arm64/armv7)"

python3 - "${IMAGE}" "${ROOT_DIR}/docker/README.md" "${SHORT}" <<'PY'
import base64, json, os, sys, urllib.request

image, readme_path, short = sys.argv[1:4]
user, token = os.environ.get("DOCKERHUB_USERNAME"), os.environ.get("DOCKERHUB_TOKEN")
if not (user and token):
    config = json.load(open(os.path.expanduser("~/.docker/config.json")))
    auth = config["auths"]["https://index.docker.io/v1/"]["auth"]
    user, token = base64.b64decode(auth).decode().split(":", 1)

def call(url, body, method):
    request = urllib.request.Request(
        url, data=json.dumps(body).encode(), method=method,
        headers={"Content-Type": "application/json"} | ({"Authorization": f"JWT {jwt}"} if method == "PATCH" else {}))
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)

jwt = call("https://hub.docker.com/v2/users/login", {"username": user, "password": token}, "POST")["token"]
readme = open(readme_path, encoding="utf-8").read()
if len(readme.encode()) > 25000:
    sys.exit("the overview is over Docker Hub's 25000 byte limit")
result = call(f"https://hub.docker.com/v2/repositories/{image}/", {"full_description": readme, "description": short[:100]}, "PATCH")
print(f"updated {result['namespace']}/{result['name']}: {len(readme)} chars, description {result['description']!r}")
PY
