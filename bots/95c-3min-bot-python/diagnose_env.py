"""
diagnose_env.py — Diagnóstico DETALHADO do arquivo .env

Verifica EXATAMENTE o que está errado com suas configurações.
"""
import os
import sys
from dotenv import load_dotenv

print("\n" + "="*70)
print("  🔍 DIAGNÓSTICO DETALHADO DO ARQUIVO .env")
print("="*70 + "\n")

# Carrega .env
load_dotenv()

# Lista de variáveis para verificar
vars_to_check = {
    "PRIVATE_KEY": "Obrigatório",
    "FUNDER_ADDRESS": "Obrigatório",
    "SIGNATURE_TYPE": "Obrigatório",
    "POLYMARKET_API_KEY": "Obrigatório",
    "POLYMARKET_API_SECRET": "Obrigatório",
    "POLYMARKET_API_PASSPHRASE": "Obrigatório",
    "BANKROLL": "Opcional",
    "LOOP_INTERVAL": "Opcional",
}

print("📋 1. VERIFICANDO VARIÁVEIS DE AMBIENTE\n")

issues = []
warnings = []

for var_name, importance in vars_to_check.items():
    value = os.getenv(var_name, "")
    
    if not value:
        if importance == "Obrigatório":
            print(f"   ❌ {var_name}: NÃO CONFIGURADA")
            issues.append(f"{var_name} está vazia")
        else:
            print(f"   ⚠️  {var_name}: Não configurada (opcional)")
            warnings.append(f"{var_name} não configurada")
        continue
    
    # Verifica problemas comuns
    has_quotes = value.startswith('"') or value.startswith("'")
    has_spaces = value != value.strip()
    
    # Mostra valor (mascarado)
    if len(value) > 20:
        display = f"{value[:8]}...{value[-6:]}"
    else:
        display = value
    
    status = "✅"
    problems = []
    
    if has_quotes:
        status = "⚠️"
        problems.append("TEM ASPAS (vai causar erro!)")
    
    if has_spaces:
        status = "⚠️"
        problems.append("TEM ESPAÇOS ANTES/DEPOIS")
    
    # Validações específicas
    if var_name == "PRIVATE_KEY":
        # Remove aspas para validar
        clean_value = value.strip('"').strip("'")
        if not clean_value.startswith("0x"):
            if len(clean_value) == 64:  # Hex sem 0x
                problems.append("Falta '0x' no início (será adicionado automaticamente)")
            elif len(clean_value) != 66:
                status = "❌"
                problems.append(f"Tamanho incorreto ({len(clean_value)} chars, esperado 66 com '0x')")
    
    if var_name == "FUNDER_ADDRESS":
        clean_value = value.strip('"').strip("'")
        if not clean_value.startswith("0x"):
            status = "❌"
            problems.append("Deve começar com '0x'")
        elif len(clean_value) != 42:
            status = "❌"
            problems.append(f"Tamanho incorreto ({len(clean_value)} chars, esperado 42)")
    
    if var_name in ["POLYMARKET_API_KEY", "POLYMARKET_API_SECRET", "POLYMARKET_API_PASSPHRASE"]:
        if has_quotes:
            issues.append(f"{var_name} tem aspas (REMOVA as aspas!)")
    
    print(f"   {status} {var_name}: {display}")
    
    if problems:
        for problem in problems:
            print(f"      ⮡ {problem}")

print("\n" + "="*70)
print("  📊 RESUMO")
print("="*70 + "\n")

if issues:
    print("❌ PROBLEMAS CRÍTICOS ENCONTRADOS:\n")
    for i, issue in enumerate(issues, 1):
        print(f"   {i}. {issue}")
    print()
else:
    print("✅ Nenhum problema crítico encontrado!\n")

if warnings:
    print("⚠️  AVISOS:\n")
    for i, warning in enumerate(warnings, 1):
        print(f"   {i}. {warning}")
    print()

print("="*70)
print("  💡 SOLUÇÕES")
print("="*70 + "\n")

# Verifica se tem aspas nas API Keys
api_key = os.getenv("POLYMARKET_API_KEY", "")
api_secret = os.getenv("POLYMARKET_API_SECRET", "")
api_pass = os.getenv("POLYMARKET_API_PASSPHRASE", "")

has_any_quotes = (
    (api_key and (api_key.startswith('"') or api_key.startswith("'"))) or
    (api_secret and (api_secret.startswith('"') or api_secret.startswith("'"))) or
    (api_pass and (api_pass.startswith('"') or api_pass.startswith("'")))
)

if has_any_quotes:
    print("🔧 PROBLEMA: API Keys com aspas\n")
    print("   Seu .env tem isso:")
    print('   POLYMARKET_API_KEY="019c929c-..."  ❌ ERRADO!')
    print()
    print("   Deve ficar assim (SEM ASPAS):")
    print('   POLYMARKET_API_KEY=019c929c-...    ✅ CORRETO!')
    print()
    print("   📝 AÇÃO: Execute este comando para corrigir automaticamente:")
    print("      python fix_env.py")
    print()

if not api_key or not api_secret or not api_pass:
    print("🔧 PROBLEMA: API Keys não configuradas\n")
    print("   ⚠️  ATENÇÃO: Você precisa pegar as API Keys no LOCAL CORRETO!")
    print()
    print("   ❌ NÃO USE: Builder API (console.polymarket.com)")
    print("   ✅ USE: Trading API (polymarket.com/settings)")
    print()
    print("   📋 PASSO A PASSO CORRETO:")
    print()
    print("   1. Acesse: https://polymarket.com/")
    print("   2. Conecte sua carteira (canto superior direito)")
    print("   3. Clique no ícone de perfil → Settings")
    print("   4. No menu lateral: API")
    print("   5. Clique em 'Create API Key'")
    print("   6. COPIE os 3 valores (Key, Secret, Passphrase)")
    print("   7. Cole no .env SEM ASPAS:")
    print()
    print("      POLYMARKET_API_KEY=019c929c-0ab7-...")
    print("      POLYMARKET_API_SECRET=miLSVQHBprwGX...")
    print("      POLYMARKET_API_PASSPHRASE=ca4446db05f7...")
    print()

print("="*70)
print("  🧪 PRÓXIMO PASSO")
print("="*70 + "\n")

if has_any_quotes:
    print("1. Execute: python fix_env.py")
    print("2. Execute: python test_auth.py")
elif not issues:
    print("Execute: python test_auth.py")
else:
    print("1. Corrija os problemas listados acima")
    print("2. Execute este script novamente: python diagnose_env.py")
    print("3. Quando estiver tudo OK, execute: python test_auth.py")

print()
