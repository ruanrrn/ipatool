import { ref, computed } from 'vue'
import { useDebounceFn } from '@vueuse/core'
import { API_BASE } from '../config.js'
import { apiFetch } from '../utils/api.js'

/**
 * 进程内搜索结果缓存。
 * key = `${region}:${normalizedQuery}`，value = { results, ts }。
 * TTL 内重复搜索直接返回缓存，避免重复请求 iTunes Search API。
 */
const SEARCH_CACHE_TTL_MS = 60_000 // 1 分钟
const SEARCH_CACHE_MAX_ENTRIES = 30
const _searchCache = new Map()

function searchCacheKey(region, query) {
  return `${region || 'US'}:${String(query).trim().toLowerCase()}`
}

/** 最近搜索记录（纯本地，非持久化，key 同缓存）。 */
const SEARCH_HISTORY_MAX = 10
const _searchHistory = ref([])

export function getSearchHistory() {
  return _searchHistory.value
}

export function clearSearchHistory() {
  _searchHistory.value = []
}

/**
 * Search composable for managing app search functionality.
 * Handles search queries, debounced search, results display,
 * and purchase status checking for search results.
 *
 * @param {Ref<Array>} accounts - The accounts ref
 * @param {Ref<number|null>} selectedAccount - The selected account index ref
 * @param {Function} onAppSelected - Callback when an app is selected (emit('app-selected', app))
 * @param {Object} options - Configuration options
 * @param {number} options.debounceDelay - Debounce delay in ms (default: 300)
 * @returns {Object} Search state and methods
 */
export function useSearch(accounts, selectedAccount, onAppSelected, options = {}) {
  const { debounceDelay = 300 } = options

  // State
  const searchQuery = ref('')
  const searchResults = ref([])
  const searchResultPurchaseStatusMap = ref({})
  const searching = ref(false)
  const currentSearchRequestId = ref(0)

  // Auto-detect: pure digits -> App ID mode
  const isAppIdInput = computed(() => /^\d+$/.test(searchQuery.value.trim()))

  // Format file size helper
  const formatFileSize = (bytes) => {
    if (!bytes) return ''
    const units = ['B', 'KB', 'MB', 'GB']
    let value = Number(bytes)
    if (!Number.isFinite(value) || value <= 0) return ''
    let unitIndex = 0
    while (value >= 1024 && unitIndex < units.length - 1) {
      value /= 1024
      unitIndex += 1
    }
    return `${value.toFixed(value >= 100 || unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`
  }

  // Debounced search handler
  const handleSearch = useDebounceFn(async () => {
    const query = searchQuery.value.trim()
    if (!query) {
      searchResults.value = []
      searchResultPurchaseStatusMap.value = {}
      return
    }

    // In App ID mode (pure digits), don't auto-search — user clicks confirm
    if (/^\d+$/.test(query)) {
      return
    }

    // Check if account is selected
    if (accounts.value.length === 0 || selectedAccount.value === '' || selectedAccount.value === null) {
      searchResults.value = []
      searchResultPurchaseStatusMap.value = {}
      return
    }

    // Clear selected app when searching for new apps
    if (onAppSelected) {
      onAppSelected(null)
    }

    const requestId = ++currentSearchRequestId.value
    searchResultPurchaseStatusMap.value = {}
    searching.value = true
    try {
      // Get current account's region
      const account = accounts.value[selectedAccount.value]
      const region = account?.region || 'US'
      const cacheKey = searchCacheKey(region, query)
      let nextResults = []

      // 缓存命中：TTL 内直接复用，跳过网络请求
      const cachedEntry = _searchCache.get(cacheKey)
      const now = Date.now()
      if (cachedEntry && now - cachedEntry.ts < SEARCH_CACHE_TTL_MS) {
        nextResults = cachedEntry.results
        if (requestId !== currentSearchRequestId.value) return
        searchResults.value = nextResults
      } else {
        // Check if it's a numeric App ID
        if (/^\d+$/.test(query)) {
          // Direct App ID lookup
          const { data } = await apiFetch(`${API_BASE}/app-meta?appid=${encodeURIComponent(query)}&region=${encodeURIComponent(region)}`, { credentials: 'include' })
          if (data.ok && data.data) {
            nextResults = [data.data]
          }
        } else {
          // Search by name or bundle ID
          const { data } = await apiFetch(`${API_BASE}/search?term=${encodeURIComponent(query)}&region=${encodeURIComponent(region)}&media=software&limit=10`, { credentials: 'include' })
          if (data.ok) {
            nextResults = data.data || []
          }
        }

        if (requestId !== currentSearchRequestId.value) {
          return
        }

        searchResults.value = nextResults

        // 仅缓存公共元数据，购买状态必须按当前账号重新检测。
        if (nextResults.length > 0) {
          _searchCache.set(cacheKey, {
            results: nextResults,
            ts: Date.now(),
          })
          // LRU 淘汰
          if (_searchCache.size > SEARCH_CACHE_MAX_ENTRIES) {
            const firstKey = _searchCache.keys().next().value
            _searchCache.delete(firstKey)
          }
        }
      }

      await refreshSearchResultsPurchaseStatus(nextResults, account?.token, requestId)
      if (requestId !== currentSearchRequestId.value) return

      // 记录搜索历史（去重，移到最前；仅在文字搜索且有结果时）
      if (!/^\d+$/.test(query) && nextResults.length > 0) {
        const lower = query.trim().toLowerCase()
        const idx = _searchHistory.value.findIndex(h => h.query.toLowerCase() === lower && h.region === region)
        if (idx >= 0) _searchHistory.value.splice(idx, 1)
        _searchHistory.value.unshift({ query: query.trim(), region, ts: Date.now() })
        if (_searchHistory.value.length > SEARCH_HISTORY_MAX) {
          _searchHistory.value.length = SEARCH_HISTORY_MAX
        }
      }

    } catch (error) {
      console.error('Search failed:', error)
      if (requestId === currentSearchRequestId.value) {
        searchResults.value = []
        searchResultPurchaseStatusMap.value = {}
      }
    } finally {
      if (requestId === currentSearchRequestId.value) {
        searching.value = false
      }
    }
  }, debounceDelay)

  // Select an app from search results
  const selectApp = (app) => {
    if (onAppSelected) {
      onAppSelected(app)
    }
    searchQuery.value = ''
    // Don't clear searchResults or searchResultPurchaseStatusMap to keep list visible
  }

  // Confirm direct App ID input
  const confirmDirectAppId = () => {
    const appId = searchQuery.value.trim()
    if (/^\d+$/.test(appId)) {
      // Create a minimal app object with just the App ID
      const minimalApp = {
        trackId: appId,
        trackName: `App ID: ${appId}`,
        artistName: '未知开发者',
        bundleId: 'unknown.bundle',
        artworkUrl60: null,
        artworkUrl100: null,
        version: '未知',
        isDirectAppId: true // Flag to indicate this is a direct App ID input
      }
      if (onAppSelected) {
        onAppSelected(minimalApp)
      }
      searchQuery.value = ''
      searchResults.value = []
    }
  }

  // Refresh purchase status for search results
  const refreshSearchResultsPurchaseStatus = async (results, token, requestId = currentSearchRequestId.value) => {
    const accountIndex = selectedAccount.value
    const isCurrent = () => requestId === currentSearchRequestId.value &&
      selectedAccount.value === accountIndex && accounts.value[accountIndex]?.token === token
    if (!Array.isArray(results) || results.length === 0 || !token) {
      if (requestId === currentSearchRequestId.value) {
        searchResultPurchaseStatusMap.value = {}
      }
      return
    }

    const appids = results.map(app => String(app?.trackId || '')).filter(Boolean)
    if (appids.length === 0) {
      if (requestId === currentSearchRequestId.value) {
        searchResultPurchaseStatusMap.value = {}
      }
      return
    }

    try {
      const { response, data } = await apiFetch(`${API_BASE}/purchase-status-batch`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify({ token, appids })
      }, { timeout: 240000, retries: 0 })
      if (!isCurrent()) return

      if (response?.ok !== false && data?.ok && data?.data?.results) {
        const batchResults = data.data.results
        const newEntries = {}
        for (const appid of appids) {
          const payload = batchResults[appid]
          if (payload) {
            const error = payload.error || (payload.status === 'error' ? '检测失败' : null)
            newEntries[appid] = {
              purchased: !error && !!payload.purchased,
              needsPurchase: !error && !!payload.needsPurchase,
              status: error ? 'error' : payload.status || 'unknown',
              error
            }
          } else {
            newEntries[appid] = {
              purchased: false,
              needsPurchase: false,
              status: 'error',
              error: '未返回状态'
            }
          }
        }
        searchResultPurchaseStatusMap.value = {
          ...searchResultPurchaseStatusMap.value,
          ...newEntries
        }
      } else {
        // Fallback: mark all as error
        const errorEntries = {}
        for (const appid of appids) {
          errorEntries[appid] = {
            purchased: false,
            needsPurchase: false,
            status: 'error',
            error: data?.error || '批量检测失败'
          }
        }
        searchResultPurchaseStatusMap.value = {
          ...searchResultPurchaseStatusMap.value,
          ...errorEntries
        }
      }
    } catch (error) {
      if (!isCurrent()) return
      const errorEntries = {}
      for (const appid of appids) {
        errorEntries[appid] = {
          purchased: false,
          needsPurchase: false,
          status: 'error',
          error: error?.message || '检测失败'
        }
      }
      searchResultPurchaseStatusMap.value = {
        ...searchResultPurchaseStatusMap.value,
        ...errorEntries
      }
    }
  }

  // Get search result category
  const getSearchResultCategory = (app) => {
    const category = app?.genres?.[0] || app?.primaryGenreName || app?.genre || app?.category
    return category || '—'
  }

  // Get search result version label
  const getSearchResultVersionLabel = (app) => {
    const version = app?.version || app?.bundle_version || app?.bundleVersion || app?.latestVersion
    if (!version) return '—'
    const text = String(version)
    return text.startsWith('v') || text.startsWith('V') ? text : `v${text}`
  }

  // Get search result size label
  const getSearchResultSizeLabel = (app) => {
    const numericCandidate = [
      app?.fileSizeBytes,
      app?.size,
      app?.bundleSizeBytes,
      app?.latestVersionSizeBytes,
      app?.latestSizeBytes
    ].find(value => Number.isFinite(Number(value)) && Number(value) > 0)

    if (numericCandidate) return formatFileSize(Number(numericCandidate))

    const textCandidate = [app?.fileSize, app?.sizeLabel, app?.formattedSize].find(Boolean)
    return textCandidate || '—'
  }

  // Get search result price label
  const getSearchResultPriceLabel = (app) => {
    const formatted = app?.formattedPrice
    if (formatted && formatted !== '0' && formatted !== '0.00') return formatted

    const price = Number(app?.price)
    if (!Number.isFinite(price)) return '价格未知'
    if (price <= 0) return '免费'
    return String(price)
  }

  // Get search result purchase state
  const getSearchResultPurchaseState = (app) => {
    const trackId = String(app?.trackId || '')
    if (!trackId) return null
    return searchResultPurchaseStatusMap.value[trackId] || null
  }

  // Get search result purchase label
  const getSearchResultPurchaseLabel = (app) => {
    const state = getSearchResultPurchaseState(app)
    if (!state) return '状态待检测'
    if (state.purchased) return Number(app?.price) > 0 ? '已购买' : '已获取'
    if (state.needsPurchase) return Number(app?.price) > 0 ? '未购买' : '未获取'
    if (state.status === 'error') return '状态异常'
    return '状态未知'
  }

  // Get search result purchase CSS class
  const getSearchResultPurchaseClass = (app) => {
    const state = getSearchResultPurchaseState(app)
    if (!state) return 'result-item__tag--muted'
    if (state.purchased) return 'result-item__tag--success'
    if (state.needsPurchase) return Number(app?.price) > 0 ? 'result-item__tag--warning' : 'result-item__tag--info'
    if (state.status === 'error') return 'result-item__tag--danger'
    return 'result-item__tag--muted'
  }

  // Check if search result is purchased
  const isSearchResultPurchased = (app) => {
    const state = getSearchResultPurchaseState(app)
    return !!state?.purchased
  }

  return {
    // State
    searchQuery,
    searchResults,
    searchResultPurchaseStatusMap,
    searching,
    isAppIdInput,
    searchHistory: _searchHistory,

    // Actions
    handleSearch,
    selectApp,
    confirmDirectAppId,
    refreshSearchResultsPurchaseStatus,
    clearSearchHistory: () => {
      _searchHistory.value = []
      _searchCache.clear()
    },

    // Helpers
    getSearchResultCategory,
    getSearchResultVersionLabel,
    getSearchResultSizeLabel,
    getSearchResultPriceLabel,
    getSearchResultPurchaseState,
    getSearchResultPurchaseLabel,
    getSearchResultPurchaseClass,
    isSearchResultPurchased,
    formatFileSize
  }
}
