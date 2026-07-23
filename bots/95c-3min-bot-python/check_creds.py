from py_clob_client.client import ClobClient
from py_clob_client.clob_types import ApiCreds
import os
import requests
from dotenv import load_dotenv

load_dotenv()

# Configuração que acreditamos ser a correta
PROXY_ADDR = "0xd604531daba1bda13e9329a6a71242e60810b3bf"
PRIVATE_KEY = os.getenv("PRIVATE_KEY", "").strip().replace('"', '')
if not PRIVATE_KEY.startswith("0x"): PRIVATE_KEY = "0x" + PRIVATE_KEY

API_KEY        = os.getenv("POLYMARKET_API_KEY", "").strip()
API_SECRET     = os.getenv("POLYMARKET_API_SECRET", "").strip()
API_PASSPHRASE = os.getenv("POLYMARKET_API_PASSPHRASE", "").strip()

print(f"--- DIAGNÓSTICO AVANÇADO ---")
print(f"Funder (Proxy): {PROXY_ADDR}")
print(f"API Key configurada: {bool(API_KEY)} (len={len(API_KEY)})")
print(f"API Secret configurado: {bool(API_SECRET)} (len={len(API_SECRET)})")
print(f"API Passphrase configurada: {bool(API_PASSPHRASE)} (len={len(API_PASSPHRASE)})")
print(f"Private Key configurada: {bool(PRIVATE_KEY)} (len={len(PRIVATE_KEY)})")

# Monkey-patch para interceptar headers
old_request = requests.Session.request
def debug_request(self, method, url, *args, **kwargs):
    print(f"\n[HTTP] {method} {url}")
    if 'headers' in kwargs:
        h = kwargs['headers']
        print("   Headers enviados:")
        for k, v in h.items():
            # Censura parcial
            if any(marker in k.lower() for marker in ['key', 'secret', 'passphrase', 'signature', 'authorization']):
                print(f"   {k}: <redacted>")
            else:
                print(f"   {k}: {v}")
    
    response = old_request(self, method, url, *args, **kwargs)
    
    print(f"   Status Code: {response.status_code}")
    if not response.ok:
        print(f"   Response Body: {response.text}")
    return response

requests.Session.request = debug_request

# Tenta autenticar
try:
    print("\nIniciando cliente...")
    creds = ApiCreds(api_key=API_KEY, api_secret=API_SECRET, api_passphrase=API_PASSPHRASE)
    
    # Tenta com Signature Type 1 e 2
    for sig_type in [1, 2]:
        print(f"\n---> Testando SIGNATURE_TYPE={sig_type}")
        c = ClobClient(
            host="https://clob.polymarket.com",
            key=PRIVATE_KEY,
            chain_id=137,
            signature_type=sig_type,
            funder=PROXY_ADDR
        )
        c.set_api_creds(creds)
        
        try:
            c.get_api_keys()
            print("✅ SUCESSO ABSOLUTO!")
            break
        except Exception as e:
            print(f"❌ Falha: {e}")
            if hasattr(e, 'response') and e.response:
                print(f"   Status: {e.response.status_code}")
                print(f"   Body: {e.response.text}")

except Exception as e:
    print(f"Erro: {e}")
