/**
 * The desktop synthetic origin (SPEC §5.2.1): serves a Reader session's app
 * and subresources from the `poid://` custom protocol.
 *
 * The reader builds the assets once (`ContainerServer.assets()` in
 * `@poid/host`); this class ships them to the Rust asset registry over IPC
 * and hands back the `poid://<session>/<entry>` URL the iframe loads. The
 * sandboxed iframe stays opaque-origin (allow-scripts, no allow-same-origin);
 * `poid://` only widens `script-src` to container content, never to the host.
 */

import type { ServedResponse, SyntheticOrigin } from "@poid/host";
import { invoke } from "@tauri-apps/api/core";

interface OriginInfo {
  base: string;
  cspSource: string;
}

interface AssetPayload {
  path: string;
  contentType: string;
  dataB64: string;
}

/** Base64-encodes bytes in chunks (spreading a whole Uint8Array into
 * `fromCharCode` overflows the argument list for large assets). */
function toBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

export class TauriOrigin implements SyntheticOrigin {
  private constructor(private readonly info: OriginInfo) {}

  /** Fetches the origin's URL base + CSP source from Rust (platform-specific:
   * `poid://localhost` everywhere except Windows, `http://poid.localhost`). */
  static async create(): Promise<TauriOrigin> {
    const info = await invoke<OriginInfo>("synthetic_origin");
    return new TauriOrigin(info);
  }

  cspAssetSource(): string {
    return this.info.cspSource;
  }

  async serve(
    sessionId: string,
    assets: Map<string, ServedResponse>,
    entryPath: string,
  ): Promise<string> {
    const payload: AssetPayload[] = [];
    for (const [path, response] of assets) {
      payload.push({ path, contentType: response.contentType, dataB64: toBase64(response.body) });
    }
    await invoke("register_session_assets", { session: sessionId, assets: payload });
    return `${this.info.base}/${sessionId}/${entryPath}`;
  }

  revoke(sessionId: string): void {
    void invoke("revoke_session_assets", { session: sessionId });
  }
}
