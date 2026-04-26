// Dark Rift — Dummy Service Worker
// ===================================
// Este SW existe ÚNICAMENTE para cumplir el requisito de los navegadores
// que exigen un Service Worker registrado para habilitar la instalación PWA.
//
// NO implementa ninguna estrategia de caché propia.
// Nginx y el script deploy_cloud.sh ya manejan el versionamiento de archivos
// con nombres físicos (dark_rift_client_{timestamp}.js / .wasm).
// Cualquier caché aquí rompería ese esquema de invalidación.

// Activar inmediatamente sin esperar a que se cierren pestañas viejas.
self.addEventListener("install", (event) => {
  self.skipWaiting();
});

// Tomar control de todas las pestañas abiertas de inmediato.
self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

// Pass-through puro: delegar todo al navegador/Nginx sin interceptar ni cachear nada.
self.addEventListener("fetch", (event) => {
  event.respondWith(fetch(event.request));
});
