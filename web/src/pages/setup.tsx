// SPDX-License-Identifier: AGPL-3.0-only
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { api, errorMessage, setCsrfToken, unwrap } from "@/api/client";
import { Button } from "@/components/ui/button";
import { CardDescription, CardTitle } from "@/components/ui/card";
import { Alert, Field, Input } from "@/components/ui/form";
import { CenteredCard } from "./login";

export function SetupPage() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [form, setForm] = useState({ token: "", email: "", display_name: "", password: "" });
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const set = (key: keyof typeof form) => (e: React.ChangeEvent<HTMLInputElement>) => setForm({ ...form, [key]: e.target.value });

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      const result = unwrap(await api.POST("/api/v1/setup", { body: form }));
      setCsrfToken(result.csrf_token ?? undefined);
      await queryClient.invalidateQueries({ queryKey: ["session"] });
      await navigate({ to: "/projects" });
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
          <CardTitle>Set up Bennghe</CardTitle>
          <CardDescription>Create the owner account. The setup token was printed by the install script on your server.</CardDescription>
        </div>
        <Field label="Setup token" htmlFor="token">
          <Input id="token" value={form.token} onChange={set("token")} autoComplete="off" required autoFocus />
        </Field>
        <Field label="Email" htmlFor="email">
          <Input id="email" type="email" value={form.email} onChange={set("email")} autoComplete="username" required />
        </Field>
        <Field label="Name" htmlFor="name">
          <Input id="name" value={form.display_name} onChange={set("display_name")} autoComplete="name" />
        </Field>
        <Field label="Password" htmlFor="password" hint="At least 12 characters.">
          <Input id="password" type="password" value={form.password} onChange={set("password")} autoComplete="new-password" minLength={12} required />
        </Field>
        {error ? <Alert>{error}</Alert> : null}
        <Button type="submit" disabled={busy}>Create owner account</Button>
      </form>
    </CenteredCard>
  );
}
