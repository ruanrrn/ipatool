import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { compileFunction } from 'node:vm'
import { createServer } from 'node:http'
import { test } from 'node:test'
import { ref, computed, watch, nextTick } from 'vue'
import { apiFetch } from '../utils/api.js'

const deferred = () => {
  let resolve
  const promise = new Promise(r => { resolve = r })
  return { promise, resolve }
}
const success = data => ({ response: { ok: true, status: 200 }, data: { ok: true, data } })
const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve() }
function load(name, apiFetch) {
  const timers = new Map()
  let timerId = 0
  let unmount
  const source = readFileSync(new URL(`../composables/${name}.js`, import.meta.url), 'utf8')
    .replace(/^import .*\n/gm, '').replace(/export function /g, 'function ')
  const dependencies = {
    ref, computed, apiFetch, API_BASE: '/api', AbortController,
    onBeforeUnmount: callback => { unmount = callback },
    setTimeout: callback => { timers.set(++timerId, callback); return timerId },
    clearTimeout: id => timers.delete(id)
  }
  const factory = compileFunction(`${source}\nreturn ${name}`, Object.keys(dependencies))(...Object.values(dependencies))
  return {
    factory, timers,
    unmount: () => unmount(),
    tick: async () => {
      const callbacks = [...timers.values()]
      timers.clear()
      await Promise.all(callbacks.map(fn => fn()))
      await flush()
    }
  }
}

test('initial polling failure retries, resets failures, then completes', async () => {
  let calls = 0
  const updates = []
  const completed = []
  const harness = load('useJobPolling', async () => {
    calls++
    if (calls === 1) throw new Error('offline')
    return success({ status: calls === 2 ? 'processing' : 'ready' })
  })
  const state = harness.factory({ onUpdate: (_, data) => updates.push(data.status), onComplete: id => completed.push(id) })
  assert.equal(state.startPolling('a'), true)
  await flush()
  assert.equal(state.getPollingStatus('a').failureCount, 1)
  assert.equal(harness.timers.size, 1)
  await harness.tick()
  assert.equal(state.getPollingStatus('a').failureCount, 0)
  await harness.tick()
  assert.deepEqual(updates, ['processing', 'ready'])
  assert.deepEqual(completed, ['a'])
  assert.equal(state.getPollingStatus('a').isActive, false)
  assert.equal(harness.timers.size, 0)
})

for (const failure of ['404', 'network']) {
  test(`polling terminates on ${failure} including initial request`, async () => {
    let calls = 0
    const interrupted = []
    const harness = load('useJobPolling', async () => {
      calls++
      if (failure === 'network') throw new Error('offline')
      return { response: { status: 404 }, data: {} }
    })
    const state = harness.factory({ maxFailures: 2, onInterrupted: (...args) => interrupted.push(args) })
    state.startPolling('a')
    await flush()
    if (failure === 'network') await harness.tick()
    assert.equal(calls, failure === 'network' ? 2 : 1)
    assert.equal(interrupted.length, 1)
    assert.equal(state.getPollingStatus('a').isActive, false)
    assert.equal(harness.timers.size, 0)
  })
}

test('polling reserves task synchronously, serializes requests, and discards stopped generation', async () => {
  const requests = []
  const updates = []
  const harness = load('useJobPolling', (_url, options) => {
    const pending = deferred()
    requests.push({ ...pending, signal: options.signal })
    return pending.promise
  })
  const state = harness.factory({ onUpdate: (_, data) => updates.push(data.status) })
  assert.equal(state.startPolling('a'), true)
  assert.equal(state.startPolling('a'), false)
  assert.equal(requests.length, 1)
  assert.equal(harness.timers.size, 0)
  state.stopPolling('a')
  assert.equal(requests[0].signal.aborted, true)
  state.startPolling('a')
  requests[1].resolve(success({ status: 'processing' }))
  await flush()
  requests[0].resolve(success({ status: 'ready' }))
  await flush()
  assert.deepEqual(updates, ['processing'])
  assert.equal(harness.timers.size, 1)
  const tick = harness.tick()
  assert.equal(requests.length, 3)
  assert.equal(harness.timers.size, 0)
  harness.unmount()
  requests[2].resolve(success({ status: 'ready' }))
  await tick
  assert.equal(requests[2].signal.aborted, true)
  assert.deepEqual(updates, ['processing'])
  assert.equal(harness.timers.size, 0)
  assert.equal(state.startPolling('b'), false)
})

test('syncPolling consistently uses custom task keys and stops absent tasks', async () => {
  const ids = []
  const harness = load('useJobPolling', async url => {
    ids.push(url.split('jobId=')[1])
    return success({ status: 'processing' })
  })
  const state = harness.factory()
  state.syncPolling([{ job: 'custom', status: 'pending' }], 'job')
  await flush()
  assert.deepEqual(ids, ['custom'])
  assert.equal(state.getPollingStatus('custom').isActive, true)
  state.syncPolling([], 'job')
  assert.equal(harness.timers.size, 0)
})

test('pending batch is refreshed through completion and slow refreshes are deduplicated', async () => {
  let calls = 0
  const pending = deferred()
  const harness = load('useBatchTasks', async () => {
    calls++
    return calls === 1 ? success([{ id: 'batch', status: 'pending' }]) : pending.promise
  })
  const state = harness.factory()
  await state.fetchTasks()
  assert.equal(state.activeTasks.value.length, 1)
  state.startPolling()
  state.startPolling()
  assert.equal(harness.timers.size, 1)
  const tick = harness.tick()
  assert.equal(calls, 2)
  assert.equal(state.fetchTasks(), state.fetchTasks())
  assert.equal(calls, 2)
  assert.equal(harness.timers.size, 0)
  pending.resolve(success([{ id: 'batch', status: 'completed' }]))
  await tick
  assert.equal(state.activeTasks.value.length, 0)
  await harness.tick()
  assert.equal(calls, 2)
  harness.unmount()
  assert.equal(harness.timers.size, 0)
})

test('batch cleanup aborts in-flight list and items and ignores late responses', async () => {
  const requests = []
  const harness = load('useBatchTasks', (_url, options) => {
    const pending = deferred()
    requests.push({ ...pending, signal: options.signal })
    return pending.promise
  })
  const state = harness.factory()
  const list = state.fetchTasks()
  const items = state.fetchItems('batch')
  harness.unmount()
  for (const request of requests) assert.equal(request.signal.aborted, true)
  requests[0].resolve(success([{ id: 'batch', status: 'pending' }]))
  requests[1].resolve(success({ items: [{ id: 'late' }] }))
  await Promise.all([list, items])
  assert.deepEqual(state.tasks.value, [])
  assert.deepEqual(state.itemCache.value, {})
  assert.equal(harness.timers.size, 0)
})

test('API timeout covers a real stalled response body and preserves caller cancellation', async () => {
  let requests = 0
  const server = createServer((_req, res) => {
    requests++
    res.writeHead(200, { 'Content-Type': 'application/json' })
    res.write('{"ok":')
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const url = `http://127.0.0.1:${server.address().port}/stalled`
  try {
    await assert.rejects(apiFetch(url, {}, { timeout: 80 }), /Request timed out after 80ms/)
    assert.equal(requests, 1)
    const controller = new AbortController()
    const reason = new Error('caller cancelled')
    const result = apiFetch(url, { signal: controller.signal }, { timeout: 1000 })
    setTimeout(() => controller.abort(reason), 40)
    await assert.rejects(result, error => error === reason)
    assert.equal(requests, 2)
  } finally {
    server.closeAllConnections()
    await new Promise(resolve => server.close(resolve))
  }
})

test('API does not replay POST/DELETE and GET retains bounded retries', async () => {
  const originalFetch = globalThis.fetch
  try {
    for (const method of ['POST', 'DELETE', 'GET']) {
      let calls = 0
      globalThis.fetch = async () => { calls++; throw new TypeError('offline') }
      await assert.rejects(apiFetch('/offline', { method }, method === 'GET' ? { retries: 1 } : {}), /offline/)
      assert.equal(calls, method === 'GET' ? 2 : 1)
    }
  } finally {
    globalThis.fetch = originalFetch
  }
})


test('Vue queue watcher cannot swallow terminal polling callback', async () => {
  const task = ref({ id: 'a', status: 'processing' })
  const completed = []
  const harness = load('useJobPolling', async () => success({ status: 'completed', downloadUrl: '/ipa' }))
  const state = harness.factory({
    onUpdate: (_, snapshot) => { task.value = { ...task.value, ...snapshot } },
    onComplete: (id, snapshot) => completed.push([id, snapshot.downloadUrl])
  })
  const stopWatch = watch(() => task.value.status, () => state.syncPolling([task.value]))
  state.startPolling('a')
  await flush()
  await nextTick()
  assert.deepEqual(completed, [['a', '/ipa']])
  assert.equal(harness.timers.size, 0)
  stopWatch()
})

test('SSE fallback registers polling during setup, isolates task keys, and ends interrupted downloads', async () => {
  const source = readFileSync(new URL('../composables/useDownload.js', import.meta.url), 'utf8')
    .replace(/^import .*\n/gm, '').replace(/export function /g, 'function ')
  let setup = true
  let pollingOptions
  let pollingCreations = 0
  let nextId = 0
  const started = []
  const updates = []
  const queue = []
  const failures = []
  const cleanups = []
  const dependencies = {
    ref, computed, API_BASE: '/api', URL,
    window: { location: { origin: 'https://example.test' } },
    EventSource: class { constructor() { throw new Error('SSE unavailable') } },
    onBeforeUnmount: callback => { assert.equal(setup, true); cleanups.push(callback) },
    useAppStore: () => ({ updateQueueItem: (id, data) => updates.push([id, data]) }),
    useNotifications: () => ({ notifyDownloadFailed() {} }),
    useJobPolling: options => {
      assert.equal(setup, true)
      pollingCreations++
      pollingOptions = options
      return { startPolling: id => started.push(id) }
    },
    apiFetch: async () => success({ jobId: `job${++nextId}` })
  }
  const useDownload = compileFunction(`${source}\nreturn useDownload`, Object.keys(dependencies))(...Object.values(dependencies))
  const state = useDownload({
    selectedApp: ref({ trackId: 'app', trackName: 'Example' }),
    selectedAccount: ref(0), accounts: ref([{ token: 'token', email: 'user@example.test' }]),
    onDownloadStarted: item => queue.push(item),
    onDownloadFailed: error => failures.push(error)
  })
  setup = false
  await state.startDownloadWithProgress('app', '', true)
  await state.startDownloadWithProgress('app', '', true)
  assert.equal(pollingCreations, 1)
  assert.deepEqual(started, ['job1', 'job2'])
  pollingOptions.onUpdate('job1', { progress: 35, stage: 'download' })
  assert.equal(updates.every(([id]) => id === 'job1'), true)
  pollingOptions.onInterrupted('job1', '任务已不存在')
  pollingOptions.onInterrupted('job2', '轮询多次失败，请检查网络')
  assert.equal(state.downloading.value, false)
  assert.equal(queue.every(item => item.status === 'error'), true)
  assert.deepEqual(failures, ['任务已不存在', '轮询多次失败，请检查网络'])
  for (const cleanup of cleanups) cleanup()
})
