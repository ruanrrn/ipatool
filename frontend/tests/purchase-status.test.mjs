// Run: node --test frontend/tests/purchase-status.test.mjs
// Load the real composable bodies; replace only browser/API integration imports.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { compileFunction } from 'node:vm'
import { test } from 'node:test'
import { ref, computed } from 'vue'

function loadComposable(file, apiFetch) {
  const source = readFileSync(new URL(`../composables/${file}.js`, import.meta.url), 'utf8')
    .replace(/^import .*\n/gm, '')
    .replace(/export function /g, 'function ')
  const dependencies = {
    ref, computed, apiFetch, API_BASE: '/api',
    onBeforeUnmount: () => {},
    useDebounceFn: fn => fn,
    useJobPolling: () => ({ startPolling() {} }),
    useNotifications: () => ({}),
    useAppStore: () => ({}),
    console: { warn() {}, error() {} }
  }
  return compileFunction(`${source}\nreturn ${file}`, Object.keys(dependencies))(...Object.values(dependencies))
}
test('claim does not report success when the verification request fails', async () => {
  const { state } = setupDownload(async url => url.endsWith('/claim')
    ? success({ claimed: true, verified: true })
    : { response: { ok: false }, data: { ok: false, error: 'verification upstream unavailable' } })
  await assert.rejects(state.claimSelectedAppInBackground('123'), /verification upstream unavailable/)
  assert.equal(state.claimingSelectedApp.value, false)
})

test('authorization checks allow bounded first-time device proof generation without transport retries', async () => {
  let control
  const { state } = setupDownload(async (_url, _options, fetchControl) => {
    control = fetchControl
    return success(owned)
  })
  await state.refreshPurchaseStatus()
  assert.equal(control?.timeout, 240000)
  assert.equal(control?.retries, 0)
})

const success = payload => ({ response: { ok: true }, data: { ok: true, data: payload } })
const owned = { purchased: true, needsPurchase: false, status: 'owned' }
const notOwned = { purchased: false, needsPurchase: true, status: 'not_owned' }
function setupDownload(apiFetch) {
  const selectedApp = ref({ trackId: 123, price: 0 })
  const selectedAccount = ref(0)
  const accounts = ref([{ token: 'A', region: 'US' }, { token: 'B', region: 'CN' }])
  const state = loadComposable('useDownload', apiFetch)({ selectedApp, selectedAccount, accounts })
  return { state, selectedApp, selectedAccount, accounts }
}
function deferred() {
  let resolve
  const promise = new Promise(r => { resolve = r })
  return { promise, resolve }
}

for (const [name, fetchResult, expected] of [
  ['HTTP 500', () => ({ response: { ok: false }, data: { ok: false, error: 'Apple upstream 500' } }), /Apple upstream 500/],
  ['network failure', () => { throw new Error('Network unreachable') }, /Network unreachable/],
  ['payload error', () => success({ ...notOwned, status: 'error', error: 'Session expired' }), /Session expired/],
  ['unknown status', () => success({ purchased: false, needsPurchase: false, status: 'unknown' }), /状态未知|无法确认/],
  ['cross-region error', () => success({ ...notOwned, error: 'Your account is not valid for use in the Chinese store' }), /区域.*US.*切换/]
]) {
  test(`download preserves ${name} rather than claiming the free app is unowned`, async () => {
    const { state } = setupDownload(async () => fetchResult())
    await assert.rejects(state.startDownloadWithProgress('123'), expected)
    assert.equal(state.purchaseRequired.value, false)
    assert.doesNotMatch(state.logs.value, /未领取此免费应用/)
  })
}

for (const previous of [owned, notOwned]) {
  test(`refresh clears stale ${previous.status} state on failure`, async () => {
    let fail = false
    const { state } = setupDownload(async () => {
      if (fail) throw new Error('Network unavailable')
      return success(previous)
    })
    await state.refreshPurchaseStatus()
    fail = true
    await state.refreshPurchaseStatus()
    assert.equal(state.purchaseRequired.value, false)
    assert.match(state.purchaseStatusText.value, /Network unavailable/)
    assert.match(state.downloadBlockedReason.value, /Network unavailable/)
  })
}

test('refresh prioritizes cross-region error over needsPurchase', async () => {
  const { state } = setupDownload(async () => success({ ...notOwned, error: 'Account not in this store' }))
  await state.refreshPurchaseStatus()
  assert.match(state.purchaseStatusText.value, /区域.*US.*切换/)
  assert.equal(state.purchaseRequired.value, false)
})

for (const invalid of ['app', 'selection', 'token']) {
  test(`refresh invalid ${invalid} clears previous ownership gate`, async () => {
    const { state, selectedApp, selectedAccount, accounts } = setupDownload(async () => success(notOwned))
    await state.refreshPurchaseStatus()
    if (invalid === 'app') selectedApp.value = null
    if (invalid === 'selection') selectedAccount.value = null
    if (invalid === 'token') accounts.value[0].token = ''
    await state.refreshPurchaseStatus()
    assert.equal(state.purchaseRequired.value, false)
    assert.equal(state.checkingPurchaseStatus.value, false)
  })
}

test('refresh ignores an old account response and keeps newest request busy', async () => {
  const a = deferred()
  const b = deferred()
  const { state, selectedAccount } = setupDownload(url => url.includes('token=A') ? a.promise : b.promise)
  const first = state.refreshPurchaseStatus()
  selectedAccount.value = 1
  const second = state.refreshPurchaseStatus()
  a.resolve(success(notOwned))
  await first
  assert.equal(state.checkingPurchaseStatus.value, true)
  assert.equal(state.purchaseRequired.value, false)
  b.resolve(success(owned))
  await second
  assert.equal(state.purchaseStatusText.value, '当前账号已领取')
})

test('refresh ignores an old failure after the new success', async () => {
  const a = deferred()
  const { state, selectedAccount } = setupDownload(url => url.includes('token=A') ? a.promise : Promise.resolve(success(owned)))
  const first = state.refreshPurchaseStatus()
  selectedAccount.value = 1
  await state.refreshPurchaseStatus()
  a.resolve({ data: { ok: false, error: 'old error' } })
  await first
  assert.equal(state.purchaseStatusText.value, '当前账号已领取')
})

test('refresh uses the requested app ID, not a different selected app', async () => {
  let requested
  const { state } = setupDownload(async url => { requested = url; return success(owned) })
  await state.refreshPurchaseStatus('456', '789')
  assert.match(requested, /appid=456&appVerId=789/)
})

test('preflight cannot create a task after the selected target changes', async () => {
  const pending = deferred()
  let tasks = 0
  const { state, selectedApp } = setupDownload(url => {
    if (url.includes('purchase-status')) return pending.promise
    tasks++
    return Promise.resolve({ data: { ok: false, error: 'unexpected task' } })
  })
  const download = state.startDownloadWithProgress('123')
  selectedApp.value = { trackId: 456, price: 0 }
  pending.resolve(success(owned))
  await assert.rejects(download, /切换|变更|重新/)
  assert.equal(tasks, 0)
})

test('task creation error wins over stale purchase-required text', async () => {
  let calls = 0
  const { state } = setupDownload(async () => {
    if (++calls === 1) return success(notOwned)
    if (calls === 2) return success(owned)
    return { data: { ok: false, needsPurchase: true, error: 'Apple license server unavailable' } }
  })
  await state.refreshPurchaseStatus()
  await assert.rejects(state.startDownloadWithProgress('123'), /Apple license server unavailable/)
})

test('genuine not_owned still asks to claim the free app', async () => {
  const { state } = setupDownload(async () => success(notOwned))
  await assert.rejects(state.startDownloadWithProgress('123'), /未领取|获取/)
  assert.equal(state.purchaseRequired.value, true)
})

test('search cache reuses metadata but rechecks current account ownership', async () => {
  let searches = 0
  const tokens = []
  const batch = deferred()
  const useSearch = loadComposable('useSearch', async (url, options) => {
    if (url.includes('/search?')) { searches++; return success([{ trackId: 123, price: 0 }]) }
    const { token } = JSON.parse(options.body)
    tokens.push(token)
    return token === 'A' ? success({ results: { 123: owned } }) : batch.promise
  })
  const accounts = ref([{ token: 'A', region: 'US' }, { token: 'B', region: 'US' }])
  const selectedAccount = ref(0)
  const state = useSearch(accounts, selectedAccount)
  state.searchQuery.value = 'cache-account-regression'
  await state.handleSearch()
  selectedAccount.value = 1
  const second = state.handleSearch()
  assert.equal(state.getSearchResultPurchaseState({ trackId: 123 }), null)
  batch.resolve(success({ results: { 123: notOwned } }))
  await second
  assert.equal(searches, 1)
  assert.deepEqual(tokens, ['A', 'B'])
  assert.equal(state.isSearchResultPurchased({ trackId: 123 }), false)
})

for (const [name, result] of [
  ['payload error with contradictory ownership flags', success({ results: { 123: { ...notOwned, status: 'error', error: 'Apple session expired' } } })],
  ['HTTP failure with a success-looking body', { response: { ok: false }, data: { ok: true, error: 'HTTP 500', data: { results: { 123: owned } } } }]
]) {
  test(`search does not classify ${name} as ownership`, async () => {
    const state = loadComposable('useSearch', async () => result)(ref([{ token: 'A' }]), ref(0))
    const app = { trackId: 123, price: 0 }
    await state.refreshSearchResultsPurchaseStatus([app], 'A')
    assert.equal(state.getSearchResultPurchaseLabel(app), '状态异常')
    assert.equal(state.isSearchResultPurchased(app), false)
    assert.equal(state.getSearchResultPurchaseState(app).needsPurchase, false)
  })
}

test('search ignores batch response after account changes without a new search', async () => {
  const pending = deferred()
  const selectedAccount = ref(0)
  const state = loadComposable('useSearch', () => pending.promise)(ref([{ token: 'A' }, { token: 'B' }]), selectedAccount)
  const refresh = state.refreshSearchResultsPurchaseStatus([{ trackId: 123 }], 'A')
  selectedAccount.value = 1
  pending.resolve(success({ results: { 123: owned } }))
  await refresh
  assert.equal(state.isSearchResultPurchased({ trackId: 123 }), false)
})
