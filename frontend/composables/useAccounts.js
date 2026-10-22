import { ref } from 'vue'
import { API_BASE } from '../config.js'
import { apiFetch } from '../utils/api.js'
import { STORAGE_KEYS } from '../utils/storage.js'

/**
 * Shared account identity key — deduplicates by email/dsid/token.
 */
export const accountIdentityKey = (acc = {}) =>
  String(acc.email || acc.dsid || acc.token || '').trim().toLowerCase()

/**
 * Deduplicate an account list by identity key.
 */
export const dedupeAccounts = (list = []) => {
  const map = new Map()
  for (const acc of list) {
    const key = accountIdentityKey(acc)
    if (!key) continue
    map.set(key, acc)
  }
  return [...map.values()]
}

/**
 * Normalise a raw server account object into the canonical local shape.
 */
const normaliseAccount = (acc) => ({
  token: acc.token,
  email: acc.email,
  dsid: acc.dsid,
  region: acc.region || 'US',
  hasSavedCredentials: !!acc.hasSavedCredentials,
  loggedInAt: acc.loggedInAt ?? null,
})

// --- Singleton state (shared across all consumers) ---
const accounts = ref([])

/**
 * Save current accounts list to localStorage and optionally notify parent.
 */
function persistAccounts() {
  accounts.value = dedupeAccounts(accounts.value)
  localStorage.setItem(STORAGE_KEYS.ACCOUNTS, JSON.stringify(accounts.value))
}

/**
 * Load accounts from server. Sessions are persisted server-side, so no
 * auto-login is attempted (accounts with 2FA cannot log in without a code).
 */
async function loadAccounts() {
  // Optimistic local load first
  const saved = localStorage.getItem(STORAGE_KEYS.ACCOUNTS)
  if (saved) {
    try { accounts.value = dedupeAccounts(JSON.parse(saved)) } catch { accounts.value = [] }
  }

  try {
    const { data } = await apiFetch(`${API_BASE}/accounts`, { credentials: 'include' })

    if (data.ok && data.data && data.data.length > 0) {
      accounts.value = dedupeAccounts(data.data.map(normaliseAccount))
      persistAccounts()
    } else if (data.ok) {
      accounts.value = []
      persistAccounts()
    }
  } catch (error) {
    console.error('Failed to load accounts from server:', error)
  }
}

/**
 * Load saved credentials list (just metadata, no secrets).
 */
async function loadSavedCredentials() {
  try {
    const { data } = await apiFetch(`${API_BASE}/credentials`, { credentials: 'include' })
    return (data.ok && data.data) ? data.data : []
  } catch (error) {
    console.error('Failed to load saved credentials:', error)
    return []
  }
}

/**
 * Composable: provides shared reactive accounts list and helpers.

 */
export function useAccounts(_options = {}) {
  return {
    accounts,
    loadAccounts,
    loadSavedCredentials,
    persistAccounts,
    accountIdentityKey,
    dedupeAccounts,
    normaliseAccount,
  }
}
