/** Same-origin deployment paths shared by the browser and development proxy. */
export function normalizeApiBasePath(value) {
    const path = (value?.trim() || '/api').replace(/\/+$/, '');
    if (!/^\/(?:[A-Za-z0-9_-]+\/)*[A-Za-z0-9_-]+$/.test(path)) {
        throw new Error('VITE_API_BASE_PATH must be an absolute same-origin path, e.g. /zjudss/api');
    }
    return path;
}
export function apiProxyPath(path, base) {
    if (path === base || path.startsWith(`${base}/`) || path.startsWith(`${base}?`)) {
        return `/api${path.slice(base.length)}`;
    }
    return path;
}
