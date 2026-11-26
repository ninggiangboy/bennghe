// SPDX-License-Identifier: AGPL-3.0-only
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/api/client";
import { ReauthProvider, useReauth } from "./reauth";

beforeAll(() => {
  // jsdom does not implement the modal dialog API.
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute("open", "");
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute("open");
  };
});

afterEach(() => vi.unstubAllGlobals());

function Harness({ action }: { action: () => Promise<string> }) {
  const withReauth = useReauth();
  return <button onClick={() => void withReauth(action).then((v) => (document.title = v))}>run</button>;
}

describe("re-authentication", () => {
  it("asks for the password after REAUTH_REQUIRED and then repeats the action once", async () => {
    const reauthBodies: unknown[] = [];
    vi.stubGlobal("fetch", async (req: Request) => {
      reauthBodies.push(await req.json());
      return new Response(null, { status: 204 });
    });
    let calls = 0;
    const action = vi.fn(async () => {
      calls += 1;
      if (calls === 1) throw new ApiError(403, "REAUTH_REQUIRED", "confirm");
      return "done";
    });

    render(
      <ReauthProvider totpEnabled={false}>
        <Harness action={action} />
      </ReauthProvider>,
    );
    await userEvent.click(screen.getByText("run"));
    await userEvent.type(await screen.findByLabelText("Password"), "hunter2hunter2");
    await userEvent.click(screen.getByText("Confirm"));

    await waitFor(() => expect(document.title).toBe("done"));
    expect(action).toHaveBeenCalledTimes(2);
    expect(reauthBodies).toEqual([{ password: "hunter2hunter2" }]);
  });

  it("does not intercept other errors", async () => {
    const action = vi.fn(async () => {
      throw new ApiError(403, "FORBIDDEN", "no");
    });
    const errors: unknown[] = [];
    function Probe() {
      const withReauth = useReauth();
      return <button onClick={() => void withReauth(action).catch((e: unknown) => errors.push(e))}>go</button>;
    }
    render(
      <ReauthProvider totpEnabled={false}>
        <Probe />
      </ReauthProvider>,
    );
    await userEvent.click(screen.getByText("go"));
    await waitFor(() => expect(errors).toHaveLength(1));
    expect(screen.queryByLabelText("Password")).toBeNull();
    expect(action).toHaveBeenCalledTimes(1);
  });
});
