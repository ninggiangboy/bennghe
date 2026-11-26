// SPDX-License-Identifier: AGPL-3.0-only
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { api, errorMessage, setCsrfToken, unwrap } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Card, CardDescription, CardTitle } from "@/components/ui/card";
import { Alert, Field, Input } from "@/components/ui/form";

export function CenteredCard({ children }: { children: React.ReactNode }) {
  return (
    <main className="mx-auto grid min-h-screen w-full max-w-sm content-center gap-4 px-4">
      <Card>{children}</Card>
    </main>
  );
}

export function LoginPage() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [step, setStep] = useState<"password" | "code">("password");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);

  const signedIn = async (csrf: string | null | undefined) => {
    setCsrfToken(csrf ?? undefined);
    await queryClient.invalidateQueries({ queryKey: ["session"] });
    await navigate({ to: "/projects" });
  };

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      if (step === "password") {
        const result = unwrap(await api.POST("/api/v1/auth/login", { body: { email, password } }));
        if (result.status === "totp_required") setStep("code");
        else await signedIn(result.csrf_token);
      } else {
        const result = unwrap(await api.POST("/api/v1/auth/login/totp", { body: { code } }));
        await signedIn(result.csrf_token);
      }
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <CenteredCard>
      <form onSubmit={submit} className="grid gap-4">
        <div>
          <CardTitle>Sign in to Bennghe</CardTitle>
          <CardDescription>{step === "password" ? "Use the account an owner created for you." : "Enter the code from your authenticator app, or a recovery code."}</CardDescription>
        </div>
        {step === "password" ? (
          <>
            <Field label="Email" htmlFor="email">
              <Input id="email" type="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} required autoFocus />
            </Field>
            <Field label="Password" htmlFor="password">
              <Input id="password" type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} required />
            </Field>
          </>
        ) : (
          <Field label="Code" htmlFor="code">
            <Input id="code" inputMode="text" autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} required autoFocus />
          </Field>
        )}
        {error ? <Alert>{error}</Alert> : null}
        <Button type="submit" disabled={busy}>{step === "password" ? "Sign in" : "Verify"}</Button>
      </form>
    </CenteredCard>
  );
}
