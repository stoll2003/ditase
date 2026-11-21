"use client";

import { MagnifyingGlass, X } from "@phosphor-icons/react";
import { useId } from "react";
import { plataforma } from "@/lib/plataforma";
import { cn } from "@/lib/utils";

const control =
  "w-full bg-foreground/[0.06] text-sm text-foreground outline-none transition-[background-color] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] placeholder:text-muted-foreground hover:bg-foreground/[0.08] focus-visible:bg-foreground/[0.1] disabled:opacity-40 aria-invalid:bg-negative/10";

const controlWin =
  "w-full bg-(--campo) border border-(--campo-borda) border-b-(--campo-linha) text-sm text-foreground outline-none transition-[border-color,background-color] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] placeholder:text-muted-foreground focus-visible:border-b-2 focus-visible:border-b-brand disabled:opacity-40 aria-invalid:border-b-negative";

type FieldProps = {
  label?: React.ReactNode;
  description?: React.ReactNode;
  /** Error text; marks the control invalid and is announced with it. */
  error?: React.ReactNode;
  className?: string;
};

/** Shared label / description / error wiring for every field. */
function Field({
  label,
  description,
  error,
  className,
  children,
}: FieldProps & {
  children: (ids: Record<string, string | undefined>) => React.ReactNode;
}) {
  const id = useId();
  const descriptionId = description ? `${id}-description` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      {label && (
        <label htmlFor={id} className="px-1 text-[0.8125rem] font-medium">
          {label}
        </label>
      )}
      {children({
        id,
        "aria-describedby":
          [descriptionId, errorId].filter(Boolean).join(" ") || undefined,
        "aria-invalid": error ? "true" : undefined,
      })}
      {description && !error && (
        <p id={descriptionId} className="px-1 text-xs text-muted-foreground">
          {description}
        </p>
      )}
      {error && (
        <p id={errorId} className="px-1 text-xs text-negative">
          {error}
        </p>
      )}
    </div>
  );
}

export type TextFieldProps = FieldProps &
  Omit<React.ComponentProps<"input">, "className">;

export function TextField({
  label,
  description,
  error,
  className,
  ...props
}: TextFieldProps) {
  const win = plataforma() === "windows";
  return (
    <Field label={label} description={description} error={error} className={className}>
      {(ids) => (
        <input {...ids} {...props} className={cn(win ? controlWin : control, "h-9 px-3.5", win ? "rounded-[4px]" : "rounded-full")} />
      )}
    </Field>
  );
}

export type SearchFieldProps = Omit<TextFieldProps, "type"> & {
  /** Shows a clear button while there is text. */
  onClear?: () => void;
};

export function SearchField({
  label,
  description,
  error,
  className,
  onClear,
  value,
  placeholder = "Search",
  ...props
}: SearchFieldProps) {
  const win = plataforma() === "windows";
  return (
    <Field label={label} description={description} error={error} className={className}>
      {(ids) => (
        <div className="relative">
          <MagnifyingGlass
            weight="fill"
            aria-hidden="true"
            className="pointer-events-none absolute top-1/2 left-3 size-3.5 -translate-y-1/2 text-muted-foreground"
          />
          <input
            {...ids}
            type="search"
            value={value}
            placeholder={placeholder}
            aria-label={label ? undefined : placeholder}
            {...props}
            className={cn(
              win ? controlWin : control,
              "h-9 pr-9 pl-8 [&::-webkit-search-cancel-button]:hidden",
              win ? "rounded-[4px]" : "rounded-full",
            )}
          />
          {onClear && value !== undefined && value !== "" && (
            <button
              type="button"
              aria-label="Clear search"
              onClick={onClear}
              className="absolute top-1/2 right-1.5 flex size-6 -translate-y-1/2 items-center justify-center rounded-full text-muted-foreground transition-[background-color,color,scale] duration-150 hover:bg-foreground/[0.08] hover:text-foreground active:scale-[0.92]"
            >
              <X weight="bold" className="size-3" aria-hidden="true" />
            </button>
          )}
        </div>
      )}
    </Field>
  );
}

export type TextAreaProps = FieldProps &
  Omit<React.ComponentProps<"textarea">, "className">;

export function TextArea({
  label,
  description,
  error,
  className,
  rows = 3,
  ...props
}: TextAreaProps) {
  const win = plataforma() === "windows";
  return (
    <Field label={label} description={description} error={error} className={className}>
      {(ids) => (
        <textarea
          {...ids}
          rows={rows}
          {...props}
          className={cn(win ? controlWin : control, "resize-none px-3.5 py-2.5 leading-relaxed", win ? "rounded-[4px]" : "rounded-xl")}
        />
      )}
    </Field>
  );
}
