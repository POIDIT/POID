/**
 * Service worker: precaches the whole shell (HTML, JS, CSS, the WASM
 * validation core) so the Web Reader is fully functional offline after the
 * first load. Cache-first — the page never depends on the network again;
 * a new deployment rotates VERSION, which retires the old cache.
 *
 * `__BUILD_HASH__` is replaced by scripts/build-site.mjs with a digest of the
 * built assets.
 */

const VERSION = "poid-web-__BUILD_HASH__";
const SHELL = [
  "./",
  "./index.html",
  "./app.js",
  "./styles.css",
  "./manifest.webmanifest",
  "./icons/icon.svg",
  "./wasm/poid_wasm.js",
  "./wasm/poid_wasm_bg.wasm",
];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(VERSION)
      .then((cache) => cache.addAll(SHELL))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== VERSION).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET" || !request.url.startsWith(self.location.origin)) return;
  event.respondWith(
    caches.match(request, { ignoreSearch: true }).then(
      (cached) =>
        cached ??
        fetch(request).then((response) => {
          // Cache successful same-origin responses so assets fetched before
          // the worker took control still end up offline-available.
          if (response.ok) {
            const copy = response.clone();
            caches.open(VERSION).then((cache) => cache.put(request, copy));
          }
          return response;
        }),
    ),
  );
});
