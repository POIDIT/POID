import type { Capability } from "@poid/sdk";
import { describe, expect, it } from "vitest";
import { Broker, type BrokerHandlers, type Connection, type ReaderSession } from "./broker.js";
import { InMemoryEngine } from "./engine.js";

function session(overrides: Partial<ReaderSession> = {}): ReaderSession {
  return {
    instanceId: overrides.instanceId ?? "instance-a",
    appInfo: {
      id: "com.example.x",
      name: "X",
      version: "1.0.0",
      readerVersion: "0.0.1",
      platform: "web",
      storageMode: "embedded",
    },
    capabilities:
      overrides.capabilities ?? new Set<Capability>(["app", "db.kv", "db.slots", "ui", "export"]),
    slots: overrides.slots ?? [],
    currentSlot: overrides.currentSlot ?? "",
    quotaBytes: overrides.quotaBytes ?? 64 * 1024 * 1024,
    connections: overrides.connections ?? new Map(),
  };
}

function req(id: number, method: string, params?: unknown) {
  return { poid: 1, id, method, params };
}

describe("scope is derived from the window, never the message", () => {
  it("ignores an instanceId the app puts in the message", async () => {
    const engine = new InMemoryEngine();
    const broker = new Broker(engine, {});
    broker.register("win-a", session({ instanceId: "instance-a" }));
    broker.register("win-b", session({ instanceId: "instance-b" }));

    // B stores a secret note in its own scope.
    await broker.handle("win-b", req(1, "db.kv.set", { key: "note", value: "B-private" }));

    // A tries to read B's data by naming B's instanceId in the message.
    const res = await broker.handle(
      "win-a",
      req(2, "db.kv.get", { key: "note", instanceId: "instance-b" }),
    );
    expect(res).toEqual({ poid: 1, id: 2, ok: true, result: null });

    // A only ever sees its own scope.
    await broker.handle("win-a", req(3, "db.kv.set", { key: "note", value: "A-private" }));
    const own = await broker.handle("win-a", req(4, "db.kv.get", { key: "note" }));
    expect(own).toMatchObject({ ok: true, result: "A-private" });
  });

  it("ignores a slot named in the message; the reader controls the slot", async () => {
    const engine = new InMemoryEngine();
    const broker = new Broker(engine, {});
    broker.register("win", session({ slots: ["a", "b"], currentSlot: "a" }));

    // Seed both slots directly via the reader-controlled slot switch.
    await broker.handle("win", req(1, "db.kv.set", { key: "k", value: "in-a" }));
    broker.setSlot("win", "b");
    await broker.handle("win", req(2, "db.kv.set", { key: "k", value: "in-b" }));
    broker.setSlot("win", "a");

    // The app asks for slot "b" in the message — ignored, it gets slot "a".
    const res = await broker.handle("win", req(3, "db.kv.get", { key: "k", slot: "b" }));
    expect(res).toMatchObject({ ok: true, result: "in-a" });

    // And there is no method that lets the app switch slots itself.
    const current = await broker.handle("win", req(4, "db.slots.current"));
    expect(current).toMatchObject({ ok: true, result: "a" });
  });
});

describe("fail closed", () => {
  it("rejects an unknown method with NOT_AVAILABLE", async () => {
    const broker = new Broker(new InMemoryEngine(), {});
    broker.register("win", session());
    const res = await broker.handle("win", req(1, "db.kv.evil", { key: "x" }));
    expect(res).toMatchObject({ ok: false, error: { code: "NOT_AVAILABLE" } });
  });

  it("rejects an unknown field with INVALID_ARGUMENT", async () => {
    const broker = new Broker(new InMemoryEngine(), {});
    broker.register("win", session());
    const res = await broker.handle("win", req(1, "db.kv.get", { key: "x", extra: "no" }));
    expect(res).toMatchObject({ ok: false, error: { code: "INVALID_ARGUMENT" } });
  });

  it("drops a message from an unregistered window", async () => {
    const broker = new Broker(new InMemoryEngine(), {});
    const res = await broker.handle("ghost", req(1, "db.kv.get", { key: "x" }));
    expect(res).toBeNull();
  });

  it("rejects a call whose capability was not granted", async () => {
    const broker = new Broker(new InMemoryEngine(), {});
    broker.register(
      "win",
      session({ capabilities: new Set<Capability>(["app", "db.kv", "ui", "export"]) }),
    );
    // net is not granted → the method is refused before any handler runs.
    const res = await broker.handle("win", req(1, "net.fetch", { url: "https://x.test" }));
    expect(res).toMatchObject({ ok: false, error: { code: "PERMISSION_DENIED" } });
  });
});

describe("credentials never cross the boundary", () => {
  const secret = "sk-live-DEADBEEF-super-secret-key";

  function withConnection(handlers: BrokerHandlers) {
    const broker = new Broker(new InMemoryEngine(), { handlers });
    const connections = new Map<string, Connection>([
      ["conn-1", { id: "conn-1", kind: "net", secret }],
    ]);
    broker.register(
      "win",
      session({
        capabilities: new Set<Capability>(["app", "db.kv", "net", "ui", "export"]),
        connections,
      }),
    );
    return broker;
  }

  it("has no method that returns a connection secret", async () => {
    // The net handler attaches the credential itself and returns only a
    // response; the secret is never part of any result.
    const broker = withConnection({
      net: async (s) => {
        const cred = [...s.connections.values()][0]?.secret;
        expect(cred).toBe(secret); // the broker/handler can see it…
        return { status: 200, statusText: "OK", headers: {}, body: "brokered" };
      },
    });
    const res = await broker.handle("win", req(1, "net.fetch", { url: "https://api.test" }));
    expect(JSON.stringify(res)).not.toContain(secret); // …but the app never does
    expect(res).toMatchObject({ ok: true, result: { body: "brokered" } });
  });

  it("scrubs a secret out of an error message (defense in depth)", async () => {
    const broker = withConnection({
      net: async () => {
        // A careless handler leaks the secret into an error string.
        throw new Error(`upstream rejected token ${secret}`);
      },
    });
    const res = await broker.handle("win", req(1, "net.fetch", { url: "https://api.test" }));
    // Unexpected errors collapse to INTERNAL, and any known secret is redacted.
    expect(res).toMatchObject({ ok: false, error: { code: "INTERNAL" } });
    expect(JSON.stringify(res)).not.toContain(secret);
  });

  it("drops an Authorization header the app tries to set", async () => {
    let sawAuth = true;
    const broker = withConnection({
      net: async (_s, params) => {
        const init = params.init as { headers?: Record<string, string> };
        sawAuth = Object.keys(init.headers ?? {}).some((k) => k.toLowerCase() === "authorization");
        return { status: 200, statusText: "OK", headers: {}, body: "" };
      },
    });
    await broker.handle(
      "win",
      req(1, "net.fetch", {
        url: "https://api.test",
        init: { headers: { authorization: "Bearer x" } },
      }),
    );
    expect(sawAuth).toBe(false);
  });
});

describe("rate limit and quota", () => {
  it("rate-limits calls with QUOTA_EXCEEDED once the bucket empties", async () => {
    const t = 0;
    const broker = new Broker(new InMemoryEngine(), {
      rateLimit: { capacity: 3, refillPerSec: 0 },
      now: () => t,
    });
    broker.register("win", session());
    const codes: (string | undefined)[] = [];
    for (let i = 0; i < 5; i++) {
      const res = await broker.handle("win", req(i, "db.kv.list", {}));
      codes.push(
        res && "ok" in res && res.ok ? "ok" : (res as { error: { code: string } }).error.code,
      );
    }
    expect(codes).toEqual(["ok", "ok", "ok", "QUOTA_EXCEEDED", "QUOTA_EXCEEDED"]);
  });

  it("enforces the storage quota", async () => {
    const broker = new Broker(new InMemoryEngine(), {});
    broker.register("win", session({ quotaBytes: 64 }));
    const big = "x".repeat(200);
    const res = await broker.handle("win", req(1, "db.kv.set", { key: "k", value: big }));
    expect(res).toMatchObject({ ok: false, error: { code: "QUOTA_EXCEEDED" } });
  });

  it("rejects malformed keys", async () => {
    const broker = new Broker(new InMemoryEngine(), {});
    broker.register("win", session());
    const res = await broker.handle("win", req(1, "db.kv.set", { key: "has space", value: 1 }));
    expect(res).toMatchObject({ ok: false, error: { code: "INVALID_ARGUMENT" } });
  });
});

describe("app.info reflects the session, not the message", () => {
  it("returns the window's own instance identity", async () => {
    const broker = new Broker(new InMemoryEngine(), {});
    broker.register("win", session({ instanceId: "the-real-one" }));
    const res = await broker.handle("win", req(1, "app.info", { instanceId: "spoofed" }));
    expect(res).toMatchObject({ ok: true, result: { instanceId: "the-real-one" } });
  });
});
