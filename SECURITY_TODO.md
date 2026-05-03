# Security TODO

- The current deployment model assumes Kanade is reachable only through a trusted reverse proxy or a VPN/local network boundary. Before exposing it directly to untrusted networks, add mandatory authentication for `/ws` UI/node connections and the OpenHome adapter.
- If reverse-proxy authentication is used, document the required headers, WebSocket forwarding behavior, and CORS origin policy in the deployment guide.
- Consider replacing the permissive default CORS policy with an allow-list once the supported deployment origins are known.
- Keep `BIND_ADDR`/`OH_ADDR` defaults under review. Changing them may break existing LAN/VPN setups, so this remains deferred until the deployment model is finalized.