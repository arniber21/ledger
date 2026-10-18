# Move one Playwright stack off the default ports when LEDGER_E2E_SLOT is set.
# When that variable is unset, this file exports nothing and the defaults stay:
# compose project ledger, Timescale on 5433, API chosen by scripts/e2e.sh, UI the same.
# A slot is an integer. Slot 38 uses project ledger-e2e-38, Postgres 5471, API 20038, UI 21038.

if [[ -z "${LEDGER_E2E_SLOT:-}" ]]; then
  return 0
fi

if ! [[ "${LEDGER_E2E_SLOT}" =~ ^[0-9]+$ ]]; then
  printf 'e2e: LEDGER_E2E_SLOT must be an integer\n' >&2
  exit 1
fi

slot="${LEDGER_E2E_SLOT}"
export LEDGER_COMPOSE_PROJECT="ledger-e2e-${slot}"
export LEDGER_DB_PORT="$((5433 + slot))"
export DATABASE_URL="postgres://ledger:ledger_local@127.0.0.1:${LEDGER_DB_PORT}/ledger_e2e"
export LEDGER_API_PORT="$((20000 + slot))"
export LEDGER_UI_PORT="$((21000 + slot))"
export LEDGER_API_ADDR="127.0.0.1:${LEDGER_API_PORT}"
export LEDGER_API_ORIGIN="http://127.0.0.1:${LEDGER_API_PORT}"
export LEDGER_UI_URL="http://127.0.0.1:${LEDGER_UI_PORT}"
