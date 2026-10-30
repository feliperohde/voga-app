"use client"

import * as React from "react"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { useI18n } from "@/i18n"

export interface ConfirmOptions {
  title?: React.ReactNode
  description?: React.ReactNode
  confirmLabel?: React.ReactNode
  cancelLabel?: React.ReactNode
  /** Deleting or losing something: the confirm button is red. */
  destructive?: boolean
}

interface ConfirmDialogProps extends ConfirmOptions {
  open: boolean
  onConfirm: () => void
  onCancel: () => void
}

/** The app's "are you sure?" dialog. */
export function ConfirmDialog({
  open,
  onConfirm,
  onCancel,
  title,
  description,
  confirmLabel,
  cancelLabel,
  destructive,
}: ConfirmDialogProps) {
  const { t } = useI18n()
  return (
    <Dialog open={open} onOpenChange={next => !next && onCancel()}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{title ?? t("confirm.title")}</DialogTitle>
          {description && <DialogDescription>{description}</DialogDescription>}
        </DialogHeader>
        <DialogFooter className="gap-2 sm:gap-0">
          <Button variant="outline" onClick={onCancel}>
            {cancelLabel ?? t("common.cancel")}
          </Button>
          <Button variant={destructive ? "destructive" : "default"} onClick={onConfirm} autoFocus>
            {confirmLabel ?? t("confirm.ok")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

type Confirm = (options: ConfirmOptions) => Promise<boolean>

const ConfirmContext = React.createContext<Confirm>(async () => false)

/** Lets any component ask with `await confirm({...})`, in place of `window.confirm`. */
export function ConfirmProvider({ children }: { children: React.ReactNode }) {
  const [request, setRequest] = React.useState<(ConfirmOptions & { resolve: (ok: boolean) => void }) | null>(null)

  const confirm = React.useCallback<Confirm>(
    options => new Promise<boolean>(resolve => setRequest({ ...options, resolve })),
    [],
  )
  const settle = (ok: boolean) => {
    request?.resolve(ok)
    setRequest(null)
  }

  return (
    <ConfirmContext.Provider value={confirm}>
      {children}
      <ConfirmDialog {...request} open={!!request} onConfirm={() => settle(true)} onCancel={() => settle(false)} />
    </ConfirmContext.Provider>
  )
}

export const useConfirm = () => React.useContext(ConfirmContext)
