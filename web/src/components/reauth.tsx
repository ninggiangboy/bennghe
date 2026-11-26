// SPDX-License-Identifier: AGPL-3.0-only
import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { ApiError, api, errorMessage, unwrap } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Alert, Field, Input } from "@/components/ui/form";

type Run = <T>(action: () => Promise<T>) => Promise<T>;

const ReauthContext = createContext<Run | null>(null);

/**
 * Sensitive operations are refused with `REAUTH_REQUIRED` when the password was not checked in
 * the last five minutes. `useReauth()` returns a wrapper that catches that, asks for the password
 * (or an authenticator code), and then repeats the operation once.
 */
export function useReauth(): Run {
  const run = useContext(ReauthContext);
  if (!run) throw new Error("useReauth must be used inside <ReauthProvider>");
  return run;
}

export function ReauthProvider({ children, totpEnabled }: { children: ReactNode; totpEnabled: boolean }) {
  const [open, setOpen] = useState(false);
  const waiting = useRef<{ resolve: () => void; reject: (e: Error) => void } | null>(null);

  const ask = useCallback(
    () =>
      new Promise<void>((resolve, reject) => {
        waiting.current = { resolve, reject };
        setOpen(true);
      }),
    [],
  );

  const run: Run = useCallback(
    async (action) => {
      try {
        return await action();
      } catch (e) {
        if (e instanceof ApiError && e.code === "REAUTH_REQUIRED") {
          await ask();
          return await action();
        }
        throw e;
      }
    },
    [ask],
  );

  const finish = (error?: Error) => {
    const w = waiting.current;
    waiting.current = null;
    setOpen(false);
    if (error) w?.reject(error);
    else w?.resolve();
  };

  return (
    <ReauthContext.Provider value={run}>
      {children}
      {open ? <ReauthDialog totpEnabled={totpEnabled} onDone={() => finish()} onCancel={() => finish(new Error("Confirmation cancelled"))} /> : null}
    </ReauthContext.Provider>
  );
}

function ReauthDialog({ totpEnabled, onDone, onCancel }: { totpEnabled: boolean; onDone: () => void; onCancel: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const [useCode, setUseCode] = useState(false);
  const [value, setValue] = useState("");
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    ref.current?.showModal();
  }, []);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      unwrap(await api.POST("/api/v1/auth/reauth", { body: useCode ? { code: value } : { password: value } }));
      onDone();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <dialog ref={ref} onCancel={onCancel} className="m-auto w-full max-w-sm rounded-lg border border-border bg-card p-6 text-foreground backdrop:bg-black/40">
      <form onSubmit={submit} className="grid gap-4">
        <div>
          <h2 className="text-base font-semibold">Confirm it is you</h2>
          <p className="mt-1 text-sm text-muted-foreground">This action needs a recent check of your {useCode ? "authenticator code" : "password"}.</p>
        </div>
        <Field label={useCode ? "Authenticator or recovery code" : "Password"} htmlFor="reauth-value">
          <Input
            id="reauth-value"
            type={useCode ? "text" : "password"}
            autoComplete={useCode ? "one-time-code" : "current-password"}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            autoFocus
            required
          />
        </Field>
        {error ? <Alert>{error}</Alert> : null}
        <div className="flex items-center justify-between gap-2">
          {totpEnabled ? (
            <Button type="button" variant="ghost" size="sm" onClick={() => { setUseCode(!useCode); setValue(""); }}>
              {useCode ? "Use password" : "Use a code"}
            </Button>
          ) : (
            <span />
          )}
          <div className="flex gap-2">
            <Button type="button" variant="outline" onClick={() => ref.current?.close()}>Cancel</Button>
            <Button type="submit" disabled={busy}>Confirm</Button>
          </div>
        </div>
      </form>
    </dialog>
  );
}
