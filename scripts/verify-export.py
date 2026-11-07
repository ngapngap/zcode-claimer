#!/usr/bin/env python3
"""Validate export JSON against router provider_conns schema."""
import json, sys, pathlib

def validate(path):
    p = pathlib.Path(path)
    if not p.exists():
        print(f"LOI: khong thay file export: {path}", file=sys.stderr)
        print(f"Hay export JSON tu app (muc 3) hoac chi truyen duong dan file that.", file=sys.stderr)
        sys.exit(1)
    try:
        data = json.loads(p.read_text())
    except json.JSONDecodeError as e:
        print(f"LOI: file khong phai JSON hop le: {e}", file=sys.stderr)
        sys.exit(1)
    assert data["provider"] == "glm", "provider must be glm"
    assert data["authType"] == "oauth", "authType must be oauth"
    assert data["data"]["providerSpecificData"]["zcodeJwtToken"], "zcodeJwtToken required"
    assert data["data"]["providerSpecificData"]["zcodeJwtToken"].count(".") == 2, "JWT must have 3 parts"
    assert "X-Device-Mid" not in str(data), "device-mid should not be in export (router manages its own)"
    print(f"OK: {data['name']} provider={data['provider']} user={data['data']['providerSpecificData'].get('username')}")

if __name__ == "__main__":
    validate(sys.argv[1] if len(sys.argv) > 1 else "export/zcode-export.json")
