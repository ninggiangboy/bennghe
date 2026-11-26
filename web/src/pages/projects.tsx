// SPDX-License-Identifier: AGPL-3.0-only
import { useMutation, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api, errorMessage, unwrap } from "@/api/client";
import { projectsQuery, sessionQuery } from "@/api/session";
import { Button } from "@/components/ui/button";
import { Card, CardDescription, CardTitle } from "@/components/ui/card";
import { Alert, Field, Input } from "@/components/ui/form";

const slugify = (name: string) =>
  name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);

export function ProjectsPage() {
  const { data: session } = useSuspenseQuery(sessionQuery);
  const { data: projects } = useSuspenseQuery(projectsQuery);
  const canCreate = session?.global_role === "owner" || session?.global_role === "admin";

  return (
    <div className="grid gap-6">
      <div>
        <h1 className="text-xl font-semibold">Projects</h1>
        <p className="mt-1 text-sm text-muted-foreground">A project is a security boundary: people and API tokens see only the projects they belong to.</p>
      </div>
      {projects.length === 0 ? (
        <Card>
          <CardTitle>No projects yet</CardTitle>
          <CardDescription>{canCreate ? "Create the first one below." : "Ask an administrator to add you to a project."}</CardDescription>
        </Card>
      ) : (
        <ul className="grid gap-3">
          {projects.map((p) => (
            <li key={p.id}>
              <Card className="py-4">
                <div className="flex items-baseline justify-between gap-4">
                  <CardTitle>{p.name}</CardTitle>
                  <code className="text-xs text-muted-foreground">{p.slug}</code>
                </div>
                {p.description ? <CardDescription>{p.description}</CardDescription> : null}
              </Card>
            </li>
          ))}
        </ul>
      )}
      {canCreate ? <CreateProject /> : null}
    </div>
  );
}

function CreateProject() {
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [slugTouched, setSlugTouched] = useState(false);
  const create = useMutation({
    mutationFn: async () => unwrap(await api.POST("/api/v1/projects", { body: { name, slug } })),
    onSuccess: async () => {
      setName("");
      setSlug("");
      setSlugTouched(false);
      await queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
  });

  return (
    <Card>
      <form
        className="grid gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate();
        }}
      >
        <CardTitle>New project</CardTitle>
        <Field label="Name" htmlFor="project-name">
          <Input
            id="project-name"
            value={name}
            onChange={(e) => {
              setName(e.target.value);
              if (!slugTouched) setSlug(slugify(e.target.value));
            }}
            required
          />
        </Field>
        <Field label="Slug" htmlFor="project-slug" hint="Lowercase letters, digits and hyphens. Used in names and URLs.">
          <Input
            id="project-slug"
            value={slug}
            onChange={(e) => {
              setSlug(e.target.value);
              setSlugTouched(true);
            }}
            pattern="[a-z0-9]([a-z0-9\-]{0,38}[a-z0-9])?"
            required
          />
        </Field>
        {create.error ? <Alert>{errorMessage(create.error)}</Alert> : null}
        <div>
          <Button type="submit" disabled={create.isPending}>Create project</Button>
        </div>
      </form>
    </Card>
  );
}
