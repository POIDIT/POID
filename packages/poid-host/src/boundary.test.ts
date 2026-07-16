import type { Capability } from "@poid/sdk";
import { describe, expect, it, vi } from "vitest";
import { SandboxBridge } from "./bridge.js";
import { Broker, type ReaderSession } from "./broker.js";
import { capabilitiesFromGrant, type Grant, type ManifestFacts } from "./capabilities.js";
import { ContainerServer, injectRuntime } from "./container-server.js";
import { buildCsp, SANDBOX_TOKENS } from "./csp.js";
import { InMemoryEngine } from "./engine.js";
import { guardRequest, SCOPE_FIELDS } from "./guard.js";
import { Watchdog, type WatchdogClock } from "./watchdog.js";

function session(instanceId: string): ReaderSession {
  return {
    instanceId,
    appInfo: {
      id: "com.example.x",
      name: "X",
      version: "1.0.0",
      readerVersion: "0.0.1",
      platform: "web",
      storageMode: "embedded",
    },
    capabilities: new Set<Capability>(["app", "db.kv", "db.slots", "ui", "export"]),
    slots: [],
    currentSlot: "",
    quotaBytes: 1024 * 1024,
    connections: new Map(),
  };
}

describe("CSP", () => {
  it("has the five baseline directives and denies network by default", () => {
    const csp = buildCsp();
    expect(csp).toContain("default-src 'self'");
    expect(csp).toContain("connect-src 'none'");
    expect(csp).toContain("object-src 'none'");
    expect(csp).toContain("base-uri 'none'");
    expect(csp).toContain("form-action 'none'");
  });

  it("widens connect-src only to safe approved origins", () => {
    const csp = buildCsp({ connectSrc: ["https://api.example.com"] });
    expect(csp).toContain("connect-src https://api.example.com");
    expect(csp).not.toContain("connect-src 'none'");
  });

  it("refuses to inject unsafe origins into the policy", () => {
    const csp = buildCsp({
      connectSrc: [
        "https://ok.example.com",
        "https://evil.com; script-src *",
        "not a url",
        "https://u:p@x.com",
      ],
    });
    expect(csp).toContain("connect-src https://ok.example.com");
    expect(csp).not.toContain("script-src *");
    expect(csp).not.toContain("evil.com");
  });

  it("the sandbox never grants allow-same-origin", () => {
    expect(SANDBOX_TOKENS).toBe("allow-scripts");
    expect(SANDBOX_TOKENS).not.toContain("allow-same-origin");
  });
});

describe("guard", () => {
  it("strips scope fields rather than rejecting them", () => {
    const g = guardRequest({
      poid: 1,
      id: 1,
      method: "db.kv.get",
      params: { key: "k", instanceId: "x", slot: "y", connectionId: "z" },
    });
    expect(g.ok).toBe(true);
    if (g.ok) {
      expect(g.params).toEqual({ key: "k" });
      for (const f of SCOPE_FIELDS) expect(f in g.params).toBe(false);
    }
  });

  it("rejects a genuinely unknown field", () => {
    const g = guardRequest({ poid: 1, id: 1, method: "db.kv.get", params: { key: "k", nope: 1 } });
    expect(g).toMatchObject({ ok: false, code: "INVALID_ARGUMENT" });
  });

  it("rejects a bad protocol marker and a missing id", () => {
    expect(guardRequest({ id: 1, method: "app.info" })).toMatchObject({ ok: false });
    expect(guardRequest({ poid: 1, method: "app.info" })).toMatchObject({
      ok: false,
      code: "INVALID_ARGUMENT",
    });
  });
});

describe("capabilities from grant", () => {
  const base: ManifestFacts = {
    storageMode: "embedded",
    storageSlots: false,
    profile: "web",
    permissions: {
      network: [],
      filesystem: "none",
      clipboard: false,
      print: false,
      notifications: false,
      mcp: [],
    },
  };
  const noGrant: Grant = {
    network: [],
    filesystem: false,
    clipboard: false,
    print: false,
    mcp: [],
    ai: false,
  };

  it("always includes the core document capabilities", () => {
    const caps = capabilitiesFromGrant(base, noGrant);
    expect(caps).toEqual(
      expect.arrayContaining(["app", "db.kv", "db.docs", "db.slots", "ui", "export"]),
    );
    expect(caps).not.toContain("net");
    expect(caps).not.toContain("ai");
  });

  it("grants net only when requested AND approved", () => {
    const requested: ManifestFacts = {
      ...base,
      permissions: { ...base.permissions, network: ["https://api.test"] },
    };
    expect(capabilitiesFromGrant(requested, noGrant)).not.toContain("net");
    expect(
      capabilitiesFromGrant(requested, { ...noGrant, network: ["https://api.test"] }),
    ).toContain("net");
  });

  it("grants sql when the profile asks for it", () => {
    expect(capabilitiesFromGrant({ ...base, profile: "web+sql" }, noGrant)).toContain("db.sql");
  });
});

describe("container server", () => {
  const files = new Map<string, Uint8Array>([
    ["app/index.html", new TextEncoder().encode("<!doctype html><head></head><body>hi</body>")],
    ["app/main.js", new TextEncoder().encode("console.log(1)")],
  ]);
  const bootstrap = {
    app: {
      id: "com.example.x",
      name: "X",
      version: "1.0.0",
      instanceId: "id-1",
      readerVersion: "0.0.1",
      platform: "web" as const,
      storageMode: "embedded" as const,
      slot: null,
    },
    capabilities: ["app", "db.kv"] as Capability[],
  };

  it("injects the CSP meta and the bootstrap before app scripts", () => {
    const server = new ContainerServer({
      files,
      entry: "app/index.html",
      sdkSource: "/*sdk*/",
      bootstrap,
    });
    const html = new TextDecoder().decode(server.resolve("app/index.html").body);
    expect(html).toContain("Content-Security-Policy");
    expect(html).toContain("connect-src 'none'");
    expect(html).toContain("__POID_BOOTSTRAP__");
    expect(html).toContain("/*sdk*/");
    // Bootstrap appears inside <head>, before the body content.
    expect(html.indexOf("__POID_BOOTSTRAP__")).toBeLessThan(html.indexOf("hi"));
  });

  it("serves other container files but 404s anything absent", () => {
    const server = new ContainerServer({
      files,
      entry: "app/index.html",
      sdkSource: "",
      bootstrap,
    });
    expect(server.resolve("app/main.js").status).toBe(200);
    expect(server.resolve("app/main.js").contentType).toContain("javascript");
    expect(server.resolve("data/store.json").status).toBe(404);
    expect(server.resolve("../../etc/passwd").status).toBe(404);
  });

  it("escapes </script> in the embedded bootstrap JSON", () => {
    const evil = {
      ...bootstrap,
      app: { ...bootstrap.app, name: "</script><script>alert(1)</script>" },
    };
    const html = injectRuntime("<head></head>", "", evil);
    expect(html).not.toContain("</script><script>alert(1)");
    expect(html).toContain("\\u003c");
  });
});

describe("bridge attributes messages by window, not by body", () => {
  it("routes to the session whose window sent the message", async () => {
    const broker = new Broker(new InMemoryEngine(), {});
    broker.register("win-a", session("instance-a"));
    broker.register("win-b", session("instance-b"));

    const windowA = { name: "A" };
    const windowB = { name: "B" };
    const postA = vi.fn();
    const postB = vi.fn();
    const bridge = new SandboxBridge(broker);
    bridge.register({ sessionId: "win-a", sourceWindow: windowA, post: postA });
    bridge.register({ sessionId: "win-b", sourceWindow: windowB, post: postB });

    // B stores data.
    await bridge.handleMessage({
      source: windowB,
      data: { poid: 1, id: 1, method: "db.kv.set", params: { key: "k", value: "B" } },
    });
    // A sends a message that lies (claims win-b in the body); attribution is by
    // window identity, so it is handled as A and sees nothing.
    await bridge.handleMessage({
      source: windowA,
      data: { poid: 1, id: 2, method: "db.kv.get", params: { key: "k", instanceId: "instance-b" } },
    });

    expect(postA).toHaveBeenCalledWith({ poid: 1, id: 2, ok: true, result: null });
  });

  it("drops a message from an unregistered window", async () => {
    const broker = new Broker(new InMemoryEngine(), {});
    broker.register("win-a", session("instance-a"));
    const bridge = new SandboxBridge(broker);
    const post = vi.fn();
    bridge.register({ sessionId: "win-a", sourceWindow: { name: "A" }, post });
    await bridge.handleMessage({
      source: { name: "attacker" },
      data: { poid: 1, id: 1, method: "db.kv.list", params: {} },
    });
    expect(post).not.toHaveBeenCalled();
  });
});

describe("watchdog", () => {
  function fakeClock(): WatchdogClock & { advance(ms: number): void; fire(): void } {
    let t = 0;
    let cb: (() => void) | null = null;
    return {
      now: () => t,
      setInterval: (fn) => {
        cb = fn;
        return 1;
      },
      clearInterval: () => {
        cb = null;
      },
      advance: (ms) => {
        t += ms;
      },
      fire: () => cb?.(),
    };
  }

  it("terminates an app that stops answering pings", () => {
    const clock = fakeClock();
    const onUnresponsive = vi.fn();
    const wd = new Watchdog(() => {}, onUnresponsive, { intervalMs: 1000, timeoutMs: 5000, clock });
    wd.start();
    // Five seconds pass with no pong.
    clock.advance(6000);
    clock.fire();
    expect(onUnresponsive).toHaveBeenCalledOnce();
    expect(wd.isUnresponsive).toBe(true);
  });

  it("stays alive while the app answers pings", () => {
    const clock = fakeClock();
    const onUnresponsive = vi.fn();
    let lastPing = 0;
    const wd = new Watchdog(
      (seq) => {
        lastPing = seq;
      },
      onUnresponsive,
      { intervalMs: 1000, timeoutMs: 5000, clock },
    );
    wd.start();
    for (let i = 0; i < 10; i++) {
      clock.advance(1000);
      clock.fire();
      wd.onPong(lastPing);
    }
    expect(onUnresponsive).not.toHaveBeenCalled();
    expect(wd.isUnresponsive).toBe(false);
  });
});
