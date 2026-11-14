import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Layers, Pencil, Trash2 } from "lucide-react";
import { useRef, useState } from "react";

import { commands, type CreateSide, type TemplateRow } from "../../bindings";
import { AlertDialog, Dialog } from "../../app/Dialog";
import { Button, TextField, Well } from "../../design/Styles";
import { queryKeys } from "../../lib/queryKeys";
import { ContextMenu, type MenuItem } from "../library/LibraryDialogs";
import { errorText, showError, useCreateNotice } from "./createData";

/** The name alert for saving and renaming; the name is made unique on save. */
export interface Naming {
  title: string;
  confirmTitle: string;
  name: string;
  /** Renames this template; `null` saves the side in front as a new one. */
  renaming: TemplateRow | null;
}

/**
 * The Templates icon button in the Create header (`TemplatesMenu`): start from a template, save the side in front as
 * one, or manage them. The current side's templates list first.
 */
export function TemplatesMenu({ side }: { side: CreateSide }) {
  const client = useQueryClient();
  const button = useRef<HTMLDivElement>(null);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const [naming, setNaming] = useState<Naming | null>(null);
  const [isManaging, setIsManaging] = useState(false);
  const templates = useQuery({
    queryKey: queryKeys.templates(side),
    queryFn: () => commands.templatesList(side),
    staleTime: 0,
  });
  const rows = templates.data ?? [];
  const refresh = () => client.invalidateQueries({ queryKey: ["templates"] });

  const apply = (row: TemplateRow) => {
    commands.templatesApply(row.id).then(
      (notice) => {
        if (notice) useCreateNotice.getState().show(notice);
        void client.invalidateQueries({ queryKey: queryKeys.create.all });
        void client.invalidateQueries({ queryKey: queryKeys.images.all });
      },
      (error: unknown) => {
        showError("Can't open the template", error);
      },
    );
  };

  const saveAs = () => {
    commands.templatesSuggestedName(side).then(
      (name) => {
        setNaming({ title: "Save as Template", confirmTitle: "Save", name, renaming: null });
      },
      (error: unknown) => {
        showError("Can't save the template", error);
      },
    );
  };

  const confirmName = async (naming: Naming) => {
    setNaming(null);
    try {
      if (naming.renaming) {
        await commands.templatesRename(naming.renaming.id, naming.name);
      } else {
        const outcome = await commands.templatesSave(side, naming.name);
        if (outcome.kind === "failed") useCreateNotice.getState().show(outcome.notice);
      }
    } catch (error) {
      showError(naming.renaming ? "Can't change the template" : "Can't save the template", error);
    } finally {
      void refresh();
    }
  };

  const items: (MenuItem | "divider")[] = [
    { title: "New from template", isDisabled: true, run: () => undefined },
    ...(rows.length === 0
      ? [{ title: "No templates yet", isDisabled: true, run: () => undefined }]
      : rows.map((row) => ({
          title: row.name,
          run: () => {
            apply(row);
          },
        }))),
    "divider",
    { title: "Save as Template…", run: saveAs },
    {
      title: "Manage Templates…",
      isDisabled: rows.length === 0,
      run: () => {
        setIsManaging(true);
      },
    },
  ];

  return (
    <div ref={button}>
      <Button
        variant="icon"
        icon={Layers}
        title="Templates"
        hint="Templates"
        aria-haspopup="menu"
        onClick={() => {
          void templates.refetch();
          const rect = button.current?.getBoundingClientRect();
          setMenuAt({ x: rect?.left ?? 0, y: (rect?.bottom ?? 0) + 4 });
        }}
      />
      {menuAt && (
        <ContextMenu
          x={menuAt.x}
          y={menuAt.y}
          items={items}
          onClose={() => {
            setMenuAt(null);
          }}
        />
      )}
      {naming && (
        <NameDialog
          naming={naming}
          onCancel={() => {
            setNaming(null);
          }}
          onConfirm={(name) => void confirmName({ ...naming, name })}
        />
      )}
      {isManaging && (
        <TemplatesSheet
          rows={rows}
          onUse={(row) => {
            setIsManaging(false);
            apply(row);
          }}
          onRename={(row) => {
            setNaming({ title: "Rename Template", confirmTitle: "Rename", name: row.name, renaming: row });
          }}
          onChanged={() => void refresh()}
          onDone={() => {
            setIsManaging(false);
          }}
        />
      )}
    </div>
  );
}

export function NameDialog({
  naming,
  onCancel,
  onConfirm,
}: {
  naming: Naming;
  onCancel: () => void;
  onConfirm: (name: string) => void;
}) {
  const [name, setName] = useState(naming.name);
  return (
    <Dialog title={naming.title} role="alertdialog" onCancel={onCancel}>
      <form
        className="flex flex-col gap-16"
        onSubmit={(event) => {
          event.preventDefault();
          onConfirm(name);
        }}
      >
        <div className="flex flex-col gap-6">
          <p className="m-0 text-heading font-bold">{naming.title}</p>
          <p className="m-0 text-body text-muted">
            Reference images are copied, so the template keeps working when the originals move.
          </p>
        </div>
        <TextField
          data-autofocus
          aria-label="Name"
          placeholder="Name"
          value={name}
          onChange={(event) => {
            setName(event.target.value);
          }}
        />
        <div className="flex justify-end gap-small">
          <Button variant="secondary" title="Cancel" onClick={onCancel} />
          <Button variant="primary" type="submit" title={naming.confirmTitle} />
        </div>
      </form>
    </Dialog>
  );
}

/** Every template with its inputs; use, rename or delete one (`TemplatesSheet`). */
function TemplatesSheet({
  rows,
  onUse,
  onRename,
  onChanged,
  onDone,
}: {
  rows: TemplateRow[];
  onUse: (row: TemplateRow) => void;
  onRename: (row: TemplateRow) => void;
  onChanged: () => void;
  onDone: () => void;
}) {
  const [deleting, setDeleting] = useState<TemplateRow | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const remove = (row: TemplateRow) => {
    setDeleting(null);
    commands.templatesDelete(row.id).then(onChanged, (error: unknown) => {
      setFailure(errorText(error));
    });
  };
  return (
    <Dialog title="Templates" onCancel={onDone} className="h-420 max-w-460">
      <p className="m-0 text-heading font-bold">Templates</p>
      {rows.length === 0 ? (
        <p className="m-auto text-body text-muted">
          No templates yet. Save the Create form as one from the Templates menu.
        </p>
      ) : (
        <ul className="m-0 flex flex-1 list-none flex-col gap-small overflow-y-auto p-0">
          {rows.map((row) => (
            <li key={row.id}>
              <Well className="flex items-center gap-medium p-medium">
                <div className="flex min-w-0 flex-1 flex-col gap-2">
                  <span className="truncate text-body text-text">{row.name}</span>
                  <span className="text-caption text-faint">{row.summary}</span>
                </div>
                <Button
                  variant="secondary"
                  title="Use"
                  onClick={() => {
                    onUse(row);
                  }}
                />
                <Button
                  variant="icon"
                  icon={Pencil}
                  title="Rename"
                  hint="Rename"
                  onClick={() => {
                    onRename(row);
                  }}
                />
                <Button
                  variant="icon"
                  icon={Trash2}
                  destructive
                  title="Delete"
                  hint="Delete"
                  onClick={() => {
                    setDeleting(row);
                  }}
                />
              </Well>
            </li>
          ))}
        </ul>
      )}
      <div className="flex justify-end">
        <Button variant="secondary" title="Done" onClick={onDone} />
      </div>
      {deleting && (
        <AlertDialog
          title={`Delete “${deleting.name}”?`}
          message="Its copied reference images are deleted too. Jobs already made from it keep their results."
          onCancel={() => {
            setDeleting(null);
          }}
        >
          <Button
            variant="secondary"
            title="Cancel"
            onClick={() => {
              setDeleting(null);
            }}
          />
          <Button
            variant="secondary"
            destructive
            title="Delete Template"
            onClick={() => {
              remove(deleting);
            }}
          />
        </AlertDialog>
      )}
      {failure !== null && (
        <AlertDialog
          title="Can't change the template"
          message={failure}
          onCancel={() => {
            setFailure(null);
          }}
        >
          <Button
            variant="primary"
            title="OK"
            onClick={() => {
              setFailure(null);
            }}
          />
        </AlertDialog>
      )}
    </Dialog>
  );
}
