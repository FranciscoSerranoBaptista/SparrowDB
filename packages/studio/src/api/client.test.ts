import { describe, it, expect, vi, beforeEach } from "vitest";

describe("SparrowClient", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("passes x-api-key header on hqlEval", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify({ result: [] }), { status: 200 })
    );
    const { SparrowClient } = await import("./client");
    await new SparrowClient("http://localhost:6969", "tok-abc").hqlEval("V | RETURN *");
    expect(fetchSpy).toHaveBeenCalledWith(
      "http://localhost:6969/__hql_runtime_eval",
      expect.objectContaining({
        headers: expect.objectContaining({ "x-api-key": "tok-abc" }),
      })
    );
  });

  it("passes empty x-api-key when no token", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify({ result: [] }), { status: 200 })
    );
    const { SparrowClient } = await import("./client");
    await new SparrowClient("http://localhost:6969", "").hqlEval("V | RETURN *");
    expect(fetchSpy).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({
        headers: expect.objectContaining({ "x-api-key": "" }),
      })
    );
  });

  it("sends query string in body for hqlEval", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify({}), { status: 200 })
    );
    const { SparrowClient } = await import("./client");
    await new SparrowClient("http://localhost:6969", "").hqlEval("V | RETURN id");
    const body = JSON.parse((fetchSpy.mock.calls[0][1] as RequestInit).body as string);
    expect(body.query).toBe("V | RETURN id");
  });

  it("throws on non-ok response", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response("Unauthorized", { status: 401 })
    );
    const { SparrowClient } = await import("./client");
    await expect(new SparrowClient("http://localhost:6969", "bad").hqlEval("V")).rejects.toThrow("401");
  });
});
