// SPDX-License-Identifier: AGPL-3.0-only
import { useMutation, useQuery, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import QRCode from "qrcode";
import { useEffect, useState } from "react";
import { api, errorMessage, unwrap } from "@/api/client";
import { projectsQuery, sessionQuery } from "@/api/session";
import { useReauth } from "@/components/reauth";
import { Button } from "@/components/ui/button";
import { Card, CardDescription, CardTitle } from "@/components/ui/card";
import { Alert, Field, Input } from "@/components/ui/form";

export function SecurityPage() {
  const { data: session } = useSuspenseQuery(sessionQuery);
  if (!session) return null;
  return (
    <div className="grid gap-6">
      <h1 className="text-xl font-semibold">Security</h1>
      {session.must_enrol_2fa ? (
        <Alert tone="info">This installation requires two-factor authentication for your role. Turn it on to continue.</Alert>
      ) : null}
      <TwoFactorCard enabled={session.totp_enabled} />
      {session.must_enrol_2fa ? null : <TokensCard />}
      {session.global_role === "owner" ? <PolicyCard /> : null}
    </div>
  );
}

function TwoFactorCard({ enabled }: { enabled: boolean }) {
  const queryClient = useQueryClient();
  const withReauth = useReauth();
  const [enrolment, setEnrolment] = useState<{ secret: string; otpauth_uri: string }>();
  const [recoveryCodes, setRecoveryCodes] = useState<string[]>();
  const [code, setCode] = useState("");
  const refresh = () => queryClient.invalidateQueries({ queryKey: ["session"] });

  const begin = useMutation({
    mutationFn: async () => unwrap(await api.POST("/api/v1/account/totp")),
    onSuccess: setEnrolment,
  });
  const confirm = useMutation({
    mutationFn: async () => unwrap(await api.POST("/api/v1/account/totp/confirm", { body: { code } })),
    onSuccess: async (result) => {
      setRecoveryCodes(result.recovery_codes);
      setEnrolment(undefined);
      setCode("");
      await refresh();
    },
  });
  const disable = useMutation({
    mutationFn: () => withReauth(async () => unwrap(await api.DELETE("/api/v1/account/totp"))),
    onSuccess: async () => {
      setRecoveryCodes(undefined);
      await refresh();
    },
  });

  return (
    <Card className="grid gap-4">
      <div>
        <CardTitle>Two-factor authentication</CardTitle>
        <CardDescription>{enabled ? "Sign-in asks for a code from your authenticator app." : "Add a code from an authenticator app as a second step at sign-in."}</CardDescription>
      </div>
      {recoveryCodes ? <RecoveryCodes codes={recoveryCodes} /> : null}
      {enrolment ? (
        <form
          className="grid gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            confirm.mutate();
          }}
        >
          <Qr uri={enrolment.otpauth_uri} />
          <p className="text-sm">
            Cannot scan it? Enter this key by hand: <code className="break-all">{enrolment.secret}</code>
          </p>
          <Field label="Code from the app" htmlFor="totp-code">
            <Input id="totp-code" inputMode="numeric" autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} required />
          </Field>
          <div>
            <Button type="submit" disabled={confirm.isPending}>Turn on</Button>
          </div>
        </form>
      ) : null}
      {[begin, confirm, disable].map((m, i) => (m.error ? <Alert key={i}>{errorMessage(m.error)}</Alert> : null))}
      {!enrolment && !enabled ? (
        <div>
          <Button onClick={() => begin.mutate()} disabled={begin.isPending}>Set up authenticator</Button>
        </div>
      ) : null}
      {enabled ? (
        <div>
          <Button variant="outline" onClick={() => disable.mutate()} disabled={disable.isPending}>Turn off</Button>
        </div>
      ) : null}
    </Card>
  );
}

function Qr({ uri }: { uri: string }) {
  const [src, setSrc] = useState<string>();
  useEffect(() => {
    void QRCode.toDataURL(uri, { margin: 1, width: 192 }).then(setSrc);
  }, [uri]);
  return src ? <img src={src} alt="QR code for your authenticator app" width={192} height={192} className="rounded-md bg-white p-2" /> : null;
}

function RecoveryCodes({ codes }: { codes: string[] }) {
  return (
    <div className="grid gap-2 rounded-md border border-border bg-muted p-4">
      <p className="text-sm font-medium">Save these recovery codes now. They are shown once and each works a single time.</p>
      <ul className="grid grid-cols-2 gap-1 font-mono text-sm">
        {codes.map((c) => (
          <li key={c}>{c}</li>
        ))}
      </ul>
      <div>
        <Button variant="outline" size="sm" onClick={() => void navigator.clipboard.writeText(codes.join("\n"))}>Copy</Button>
      </div>
    </div>
  );
}

function TokensCard() {
  const queryClient = useQueryClient();
  const withReauth = useReauth();
  const { data: projects } = useSuspenseQuery(projectsQuery);
  const { data: tokens } = useQuery({
    queryKey: ["api-tokens"],
    queryFn: async () => unwrap(await api.GET("/api/v1/account/api-tokens")),
  });
  const [name, setName] = useState("");
  const [role, setRole] = useState("developer");
  const [days, setDays] = useState(90);
  const [chosen, setChosen] = useState<string[]>([]);
  const [created, setCreated] = useState<string>();
  const reload = () => queryClient.invalidateQueries({ queryKey: ["api-tokens"] });

  const create = useMutation({
    mutationFn: () =>
      withReauth(async () => unwrap(await api.POST("/api/v1/account/api-tokens", { body: { name, role, project_ids: chosen, expires_in_days: days } }))),
    onSuccess: async (result) => {
      setCreated(result.token);
      setName("");
      setChosen([]);
      await reload();
    },
  });
  const revoke = useMutation({
    mutationFn: async (token_id: string) => unwrap(await api.DELETE("/api/v1/account/api-tokens/{token_id}", { params: { path: { token_id } } })),
    onSuccess: reload,
  });

  return (
    <Card className="grid gap-4">
      <div>
        <CardTitle>API tokens</CardTitle>
        <CardDescription>For CI. A token works only in the projects you pick and can never do more than you can.</CardDescription>
      </div>
      {created ? (
        <Alert tone="info">
          Copy the token now; it is not shown again: <code className="break-all">{created}</code>
        </Alert>
      ) : null}
      {tokens && tokens.length > 0 ? (
        <ul className="grid gap-2 text-sm">
          {tokens.map((t) => (
            <li key={t.id} className="flex items-center justify-between gap-2 rounded-md border border-border px-3 py-2">
              <span>
                <strong>{t.name}</strong> <span className="text-muted-foreground">…{t.hint} · {t.role} · expires {new Date(t.expires_at).toLocaleDateString()}{t.revoked_at ? " · revoked" : ""}</span>
              </span>
              {t.revoked_at ? null : (
                <Button variant="outline" size="sm" onClick={() => revoke.mutate(t.id)}>Revoke</Button>
              )}
            </li>
          ))}
        </ul>
      ) : null}
      <form
        className="grid gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate();
        }}
      >
        <Field label="Name" htmlFor="token-name">
          <Input id="token-name" value={name} onChange={(e) => setName(e.target.value)} required />
        </Field>
        <fieldset className="grid gap-1.5">
          <legend className="text-sm font-medium">Projects</legend>
          {projects.length === 0 ? <p className="text-sm text-muted-foreground">You are not in any project yet.</p> : null}
          {projects.map((p) => (
            <label key={p.id} className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={chosen.includes(p.id)} onChange={(e) => setChosen(e.target.checked ? [...chosen, p.id] : chosen.filter((x) => x !== p.id))} />
              {p.name}
            </label>
          ))}
        </fieldset>
        <div className="grid grid-cols-2 gap-4">
          <Field label="Highest role" htmlFor="token-role">
            <select id="token-role" className="h-9 rounded-md border border-border bg-card px-2 text-sm" value={role} onChange={(e) => setRole(e.target.value)}>
              <option value="viewer">viewer</option>
              <option value="developer">developer</option>
              <option value="admin">admin</option>
            </select>
          </Field>
          <Field label="Expires in (days)" htmlFor="token-days" hint="1 to 365.">
            <Input id="token-days" type="number" min={1} max={365} value={days} onChange={(e) => setDays(Number(e.target.value))} required />
          </Field>
        </div>
        {[create, revoke].map((m, i) => (m.error ? <Alert key={i}>{errorMessage(m.error)}</Alert> : null))}
        <div>
          <Button type="submit" disabled={create.isPending || chosen.length === 0}>Create token</Button>
        </div>
      </form>
    </Card>
  );
}

function PolicyCard() {
  const queryClient = useQueryClient();
  const { data } = useQuery({
    queryKey: ["security-settings"],
    queryFn: async () => unwrap(await api.GET("/api/v1/settings/security")),
  });
  const save = useMutation({
    mutationFn: async (require_2fa_for_admins: boolean) => unwrap(await api.PUT("/api/v1/settings/security", { body: { require_2fa_for_admins } })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["security-settings"] }),
  });
  return (
    <Card className="grid gap-3">
      <div>
        <CardTitle>Sign-in policy</CardTitle>
        <CardDescription>Owners and admins without two-factor authentication are sent to set it up before they can do anything else. Turn on your own first.</CardDescription>
      </div>
      <label className="flex items-center gap-2 text-sm">
        <input type="checkbox" checked={data?.require_2fa_for_admins ?? false} disabled={!data || save.isPending} onChange={(e) => save.mutate(e.target.checked)} />
        Require two-factor authentication for owners and admins
      </label>
      {save.error ? <Alert>{errorMessage(save.error)}</Alert> : null}
    </Card>
  );
}
