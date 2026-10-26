/**
 * Unified API fetch wrapper
 * Handles credentials, response parsing, timeout, retry, and error handling
 */

const DEFAULT_TIMEOUT = 30000 // 30s
const MAX_RETRIES = 2 // up to 3 total attempts
const RETRY_BACKOFF_BASE = 1000 // start at 1s

// Only these methods are safe to replay after a network error: a POST/DELETE may already have
// been applied by the server (e.g. a purchase or a download job) when the connection dropped.
const IDEMPOTENT_METHODS = new Set(['GET', 'HEAD', 'OPTIONS'])

/**
 * Fetch wrapper with timeout, retry, and credential handling
 * @param {string} url - The URL to fetch
 * @param {object} options - Fetch options (method, headers, body, etc.)
 * @param {object} [fetchOptions] - Additional fetch control
 * @param {number} [fetchOptions.timeout] - Timeout in ms (default 30000)
 * @param {number} [fetchOptions.retries] - Max retry attempts (default 2 for GET/HEAD/OPTIONS, 0 otherwise)
 * @returns {Promise<{response: Response, data: any}>} Object containing response and parsed data
 * @throws {Error} If network error or timeout
 */
export const apiFetch = async (url, options = {}, { timeout = DEFAULT_TIMEOUT, retries } = {}) => {
  const fetchOptions = {
    ...options,
    credentials: 'include',
  }
  const method = (options.method || 'GET').toUpperCase()
  if (retries === undefined) {
    retries = IDEMPOTENT_METHODS.has(method) ? MAX_RETRIES : 0
  }

  let lastError = null

  for (let attempt = 0; attempt <= retries; attempt++) {
    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(), timeout)
    fetchOptions.signal = controller.signal

    try {
      const response = await fetch(url, fetchOptions)
      clearTimeout(timer)

      let data
      try {
        data = await response.json()
      } catch {
        data = { ok: false, error: 'Invalid response format' }
      }

      return { response, data }
    } catch (err) {
      clearTimeout(timer)

      // Don't retry on abort (timeout) or if we've exhausted retries
      if (err.name === 'AbortError') {
        throw new Error(`Request timed out after ${timeout}ms`, { cause: err })
      }

      lastError = err

      // Only retry on network errors, not HTTP errors
      if (attempt < retries) {
        const delay = RETRY_BACKOFF_BASE * Math.pow(2, attempt)
        await new Promise(resolve => setTimeout(resolve, delay))
      }
    }
  }

  throw lastError || new Error('Request failed')
}
