#!/usr/bin/env python3
"""Hash a XavierDB app token with Argon2id (PHC) for authorized_keys.yml.

The plaintext token is NEVER stored — authorized_keys.yml only holds the
Argon2id `token_hash`. If a token is lost, reset it via the dashboard or
rewrite the yml entry with a freshly hashed value from this script.

Usage (uv is mandatory on this repo — a system python may be a broken stub):

    uv run --with argon2-cffi python .agents/skills/credentials/hash-token.py <token>
    echo -n "$TOKEN" | uv run --with argon2-cffi python .agents/skills/credentials/hash-token.py -

Reading the token from stdin avoids putting it in shell history / argv.

Output: a PHC string (e.g. $argon2id$v=19$m=65536,t=3,p=4$...) suitable for
the `token_hash` field.

CAVEAT: verify the hash against the SERVER (swap the token_hash in
authorized_keys.yml -> watcher reload -> /auth), NOT against argon2-cffi's
verify() — argon2-cffi verification has been observed broken in some
environments. A wrong-hash test costs ~5 s (dummy-PHC timing equalization).
"""

import sys

try:
    from argon2 import PasswordHasher
except ImportError:
    sys.stderr.write(
        "argon2-cffi not installed — run via: "
        "uv run --with argon2-cffi python .agents/skills/credentials/hash-token.py ...\n"
    )
    sys.exit(1)


def main(argv: list[str]) -> int:
    if len(argv) == 2 and argv[1] == "-":
        token = sys.stdin.read().strip()
    elif len(argv) == 2:
        token = argv[1]
    else:
        sys.stderr.write(__doc__ + "\n")
        return 2
    if not token:
        sys.stderr.write("error: empty token\n")
        return 1
    print(PasswordHasher().hash(token))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))