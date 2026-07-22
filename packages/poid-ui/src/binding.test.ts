// @vitest-environment happy-dom
import { describe, expect, it, vi } from "vitest";
import { type BindingModel, renderBindingChoice } from "./binding.js";

function stage(): HTMLElement {
  const host = document.createElement("div");
  document.body.replaceChildren(host);
  return host;
}

function model(overrides: Partial<BindingModel> = {}): BindingModel {
  return {
    appName: "Kanban",
    need: "sql",
    canConnect: true,
    candidates: [
      { id: "c1", label: "my-supabase", target: "app@db.example.com:5432/appdb", hasSecret: true },
    ],
    ...overrides,
  };
}

describe("the connection-choice prompt", () => {
  it("offers every usable connection and always offers to keep data local", () => {
    const body = stage();
    const onChoose = vi.fn();
    renderBindingChoice(
      body,
      model({
        candidates: [
          { id: "c1", label: "my-supabase", target: "a@x:5432/db", hasSecret: true },
          { id: "c2", label: "work-postgres", target: "b@y:5432/db", hasSecret: true },
        ],
      }),
      { onChoose },
    );

    const buttons = [...body.querySelectorAll("button")].map((b) => b.textContent ?? "");
    expect(buttons.some((t) => t.includes("my-supabase"))).toBe(true);
    expect(buttons.some((t) => t.includes("work-postgres"))).toBe(true);
    // SPEC §7.2.3 clause 2: declining every connection is always available.
    // A reader offering only "connect or close" turns the manifest's
    // declaration into a demand.
    expect(buttons.some((t) => t.includes("Keep the data on this computer"))).toBe(true);
  });

  it("reports the chosen connection by id, and local as local", () => {
    const body = stage();
    const onChoose = vi.fn();
    renderBindingChoice(body, model(), { onChoose });

    body.querySelector<HTMLButtonElement>(".poid-binding__candidate")?.click();
    expect(onChoose).toHaveBeenCalledWith({ kind: "connection", id: "c1" });

    const body2 = stage();
    const onChoose2 = vi.fn();
    renderBindingChoice(body2, model(), { onChoose: onChoose2 });
    const keep = [...body2.querySelectorAll("button")].find((b) =>
      b.textContent?.includes("Keep the data"),
    );
    keep?.click();
    expect(onChoose2).toHaveBeenCalledWith({ kind: "local" });
  });

  it("still offers to keep data local when nothing is configured", () => {
    const body = stage();
    renderBindingChoice(body, model({ candidates: [] }), { onChoose: vi.fn() });
    const buttons = [...body.querySelectorAll("button")].map((b) => b.textContent ?? "");
    expect(buttons.some((t) => t.includes("Keep the data on this computer"))).toBe(true);
  });

  it("lists a connection whose password has gone, without offering it", () => {
    const body = stage();
    const onChoose = vi.fn();
    renderBindingChoice(
      body,
      model({
        candidates: [{ id: "c1", label: "my-supabase", target: "a@x:5432/db", hasSecret: false }],
      }),
      { onChoose },
    );

    // Not offered as a choice...
    expect(body.querySelector(".poid-binding__candidate")).toBeNull();
    // ...but not silently dropped either, which would look like the user had
    // never configured it.
    expect(body.querySelector(".poid-binding__stale")?.textContent).toContain("my-supabase");
  });

  it("says plainly that a reader without a credential store will not ask", () => {
    const body = stage();
    const onChoose = vi.fn();
    renderBindingChoice(body, model({ canConnect: false }), { onChoose });

    const text = body.textContent ?? "";
    expect(text).toContain("cannot hold database passwords safely");
    // No connection is offered, because offering one would mean holding a
    // credential somewhere weaker than an OS credential store (SPEC §7.2.2).
    expect(body.querySelector(".poid-binding__candidate")).toBeNull();

    const only = [...body.querySelectorAll("button")];
    expect(only).toHaveLength(1);
    only[0]?.click();
    expect(onChoose).toHaveBeenCalledWith({ kind: "local" });
  });

  it("tears itself down once a choice is made", () => {
    const body = stage();
    renderBindingChoice(body, model(), { onChoose: vi.fn() });
    expect(body.querySelector(".poid-binding")).not.toBeNull();
    body.querySelector<HTMLButtonElement>(".poid-binding__candidate")?.click();
    expect(body.querySelector(".poid-binding")).toBeNull();
  });

  it("names the application and what it asked for", () => {
    const body = stage();
    renderBindingChoice(body, model({ appName: "Survey", need: "docs" }), { onChoose: vi.fn() });
    const text = body.textContent ?? "";
    expect(text).toContain("Survey");
    expect(text).toContain("external document store");
  });
});
