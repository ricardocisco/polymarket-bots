"""
generate_api_keys.py — Gera credenciais API da Polymarket

Este script DERIVA suas credenciais API usando sua PRIVATE_KEY.
As credenciais são determinísticas - sempre as mesmas para a mesma private key.

🔑 IMPORTANTE:
- NÃO existe interface web para criar API Keys
- As credenciais são GERADAS PROGRAMATICAMENTE da sua private key
- Este é o método OFICIAL da Polymarket para obter credenciais

Uso:
    python generate_api_keys.py
"""
from py_clob_client.client import ClobClient
from py_clob_client.clob_types import ApiCreds
import os
from dotenv import load_dotenv

load_dotenv()

HOST = "https://clob.polymarket.com"
CHAIN_ID = 137  # Polygon

print("\n" + "="*70)
print("  🔑 GERADOR DE CREDENCIAIS API - POLYMARKET")
print("="*70 + "\n")

# ══════════════════════════════════════════════════════════════
# 1. VALIDAÇÃO DA PRIVATE KEY
# ══════════════════════════════════════════════════════════════

PRIVATE_KEY = os.getenv("PRIVATE_KEY", "").strip().replace('"', '').replace("'", "")

if not PRIVATE_KEY:
    print("❌ ERRO: PRIVATE_KEY não encontrada no .env")
    print("\n📝 Configure PRIVATE_KEY no arquivo .env e tente novamente.\n")
    exit(1)

# Formata a private key
if not PRIVATE_KEY.startswith("0x"):
    PRIVATE_KEY = "0x" + PRIVATE_KEY

print("✅ Private Key encontrada")
print(f"   Endereço: {PRIVATE_KEY[:10]}...{PRIVATE_KEY[-6:]}")
print()

# ══════════════════════════════════════════════════════════════
# 2. CRIAÇÃO DO CLIENT E DERIVAÇÃO DAS CREDENCIAIS
# ══════════════════════════════════════════════════════════════

print("🔄 Derivando credenciais API da sua private key...")
print("   (Este processo é determinístico e sempre gera as mesmas credenciais)\n")

try:
    # Cria client APENAS com private key
    client = ClobClient(
        host=HOST,
        chain_id=CHAIN_ID,
        key=PRIVATE_KEY
    )
    
    # DERIVA ou CRIA as credenciais (método oficial)
    creds = client.create_or_derive_api_creds()
    
    print("="*70)
    print("  ✅ CREDENCIAIS GERADAS COM SUCESSO!")
    print("="*70 + "\n")
    
    print("📋 Cole estas linhas no seu arquivo .env:\n")
    print("-" * 70)
    print(f"POLYMARKET_API_KEY={creds.api_key}")
    print(f"POLYMARKET_API_SECRET={creds.api_secret}")
    print(f"POLYMARKET_API_PASSPHRASE={creds.api_passphrase}")
    print("-" * 70)
    
    print("\n" + "="*70)
    print("  📖 ENTENDENDO AS CREDENCIAIS")
    print("="*70 + "\n")
    
    print("🔐 Como funciona:")
    print("   1. Polymarket NÃO tem interface web para criar API Keys")
    print("   2. As credenciais são DERIVADAS da sua PRIVATE_KEY")
    print("   3. O processo é determinístico (sempre gera as mesmas)")
    print("   4. Cada private key tem apenas 1 conjunto de credenciais válido")
    
    print("\n📚 Documentação oficial:")
    print("   https://docs.polymarket.com/trading/clients/l1#createorderiveapikey")
    
    print("\n⚠️  NUNCA compartilhe:")
    print("   • Sua PRIVATE_KEY")
    print("   • Suas API credenciais (KEY, SECRET, PASSPHRASE)")
    
    print("\n✅ Próximos passos:")
    print("   1. Copie as credenciais acima para seu .env")
    print("   2. Execute: python test_auth.py")
    print("   3. Se tudo OK, rode o bot: python main.py")
    print()
    
except Exception as e:
    print("="*70)
    print("  ❌ ERRO AO GERAR CREDENCIAIS")
    print("="*70 + "\n")
    
    print(f"Erro: {str(e)}\n")
    
    print("🔍 Possíveis causas:")
    print("   • PRIVATE_KEY inválida ou mal formatada")
    print("   • Problemas de conexão com Polymarket")
    print("   • Falta de dependências (rode: pip install py-clob-client)")
    print()
    
    import traceback
    print("\n📋 Detalhes técnicos:")
    print(traceback.format_exc())
