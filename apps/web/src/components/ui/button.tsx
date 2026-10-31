import type { ComponentProps, ReactNode } from "react"

type ButtonProps = ComponentProps<"button"> & {
  variant?: "primary" | "secondary" | "ghost" | "danger"
}

export function Button({
  variant = "secondary",
  type = "button",
  className,
  ...props
}: ButtonProps) {
  return (
    <button
      {...props}
      type={type}
      className={["ui-button", `ui-button-${variant}`, className]
        .filter(Boolean)
        .join(" ")}
    />
  )
}

export function IconButton({
  label,
  title = label,
  variant = "ghost",
  className,
  children,
  ...props
}: Omit<ButtonProps, "aria-label"> & {
  label: string
  children: ReactNode
}) {
  return (
    <Button
      {...props}
      variant={variant}
      title={title}
      aria-label={label}
      className={["ui-icon-button", className].filter(Boolean).join(" ")}
    >
      {children}
    </Button>
  )
}
