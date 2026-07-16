import { describe, expect, it, vi } from "vitest";
import { buildPoidApi } from "./api.js";
import { PoidError } from "./errors.js";
import type { BootstrapConfig, Capability } from "./protocol.js";
import { RpcClient, type Transport } from "./rpc.js";

/** A transport wired to a scripted responder, for driving the SDK in Node. */
function harness(respond: (method: string, params: unknown) => unknown | Promise<unknown>) {
  let onMsg: ((data: unknown) => void) | undefined;
  const sent: unknown[] = [];
  const transport: Transport = {
    post: (message) => {
      sent.push(message);
      const m = message as { id?: number; method?: string; params?: unknown };
      if (m.id === undefined || m.method === undefined) return;
      Promise.resolve()
        .then(() => respond(m.method as string, m.params))
        .then(
          (result) => onMsg?.({ poid: 1, id: m.id, ok: true, result }),
          (err: unknown) =>
            onMsg?.({
              poid: 1,
              id: m.id,
              ok: false,
              error: {
                code: err instanceof PoidError ? err.code : "INTERNAL",
                message: String(err),
              },
            }),
        );
    },
    onMessage: (handler) => {
      onMsg = handler;
    },
  };
  return { transport, sent, push: (data: unknown) => onMsg?.(data) };
}

function config(capabilities: Capability[]): BootstrapConfig {
  return {
    app: {
      id: "com.example.x",
      name: "X",
      version: "1.0.0",
      instanceId: "11111111-1111-4111-8111-111111111111",
      readerVersion: "0.0.1",
      platform: "web",
      storageMode: "embedded",
      slot: null,
    },
    capabilities,
  };
}

describe("capability absence is observable", () => {
  it("omits net, ai, mcp, sql, print and clipboard when not granted", () => {
    const { transport } = harness(() => null);
    const poid = buildPoidApi(config(["app", "db.kv", "ui", "export"]), new RpcClient(transport));
    expect(poid.net).toBeUndefined();
    expect(poid.ai).toBeUndefined();
    expect(poid.mcp).toBeUndefined();
    expect(poid.db.sql).toBeUndefined();
    expect("net" in poid).toBe(false);
    expect(poid.ui.print).toBeUndefined();
    expect(poid.ui.clipboard).toBeUndefined();
  });

  it("builds net, ai, sql, clipboard and print when granted", () => {
    const { transport } = harness(() => null);
    const poid = buildPoidApi(
      config(["app", "db.kv", "db.sql", "net", "ai", "ui", "ui.print", "ui.clipboard", "export"]),
      new RpcClient(transport),
    );
    expect(typeof poid.net?.fetch).toBe("function");
    expect(typeof poid.ai?.complete).toBe("function");
    expect(typeof poid.db.sql?.exec).toBe("function");
    expect(typeof poid.ui.print).toBe("function");
    expect(typeof poid.ui.clipboard?.read).toBe("function");
  });
});

describe("rpc plumbing", () => {
  it("resolves a call with the brokered result", async () => {
    const { transport } = harness((method, params) => {
      expect(method).toBe("db.kv.get");
      expect(params).toEqual({ key: "note" });
      return "hello";
    });
    const poid = buildPoidApi(config(["app", "db.kv", "ui", "export"]), new RpcClient(transport));
    await expect(poid.db.kv.get("note")).resolves.toBe("hello");
  });

  it("rejects with a typed PoidError carrying the code", async () => {
    const { transport } = harness(() => {
      throw new PoidError("QUOTA_EXCEEDED", "over quota");
    });
    const poid = buildPoidApi(config(["app", "db.kv", "ui", "export"]), new RpcClient(transport));
    await expect(poid.db.kv.set("k", "v")).rejects.toMatchObject({
      name: "PoidError",
      code: "QUOTA_EXCEEDED",
    });
  });

  it("delivers suspend events to onSuspend handlers", () => {
    const { transport, push } = harness(() => null);
    const poid = buildPoidApi(config(["app", "db.kv", "ui", "export"]), new RpcClient(transport));
    const cb = vi.fn();
    poid.app.onSuspend(cb);
    push({ poid: 1, event: "suspend" });
    expect(cb).toHaveBeenCalledOnce();
  });

  it("auto-answers ping with pong (liveness without app cooperation)", () => {
    const { transport, sent, push } = harness(() => null);
    new RpcClient(transport);
    push({ poid: 1, ping: 7 });
    expect(sent).toContainEqual({ poid: 1, pong: 7 });
  });

  it("ignores foreign postMessage traffic", () => {
    const { transport, push } = harness(() => null);
    new RpcClient(transport);
    expect(() => push({ hello: "world" })).not.toThrow();
    expect(() => push(null)).not.toThrow();
  });
});

describe("net.fetch never forwards Authorization from the app", () => {
  it("strips an app-set Authorization header before it reaches the broker", async () => {
    let seen: unknown;
    const { transport } = harness((method, params) => {
      if (method === "net.fetch") {
        seen = params;
        return { status: 200, statusText: "OK", headers: {}, body: "ok" };
      }
      return null;
    });
    const poid = buildPoidApi(
      config(["app", "net", "ui", "db.kv", "export"]),
      new RpcClient(transport),
    );
    await poid.net?.fetch("https://api.example.com", {
      headers: { Authorization: "Bearer stolen", "X-Fine": "1" },
    });
    const init = (seen as { init: { headers: Record<string, string> } }).init;
    expect(init.headers.Authorization).toBeUndefined();
    expect(init.headers["X-Fine"]).toBe("1");
  });
});
