import React, { useEffect, useRef } from "react";
import { X } from "lucide-react";

export function Status({ value, label }: { value: string; label?: string }) {
  const labels: Record<string, string> = {
    ready: "检查可用",
    configured: "已配置",
    error: "连接异常",
    needs_auth: "需要认证",
    disconnected: "已断开",
    authorizing: "等待授权",
    applied: "待 Agent 加载",
    failed: "操作失败",
    restored: "已恢复",
    conflict: "需要核对",
    pending: "待核对",
  };
  return (
    <span className={`status ${value}`}>
      <i />
      {label || labels[value] || value}
    </span>
  );
}

export const ModalNotice = React.createContext<React.ReactNode>(null);

function isBackdrop(event: React.MouseEvent<HTMLDialogElement>) {
  const dialog = event.currentTarget;
  if (event.target !== dialog) return false;
  const box = dialog.getBoundingClientRect();
  return (
    event.clientX < box.left ||
    event.clientX > box.right ||
    event.clientY < box.top ||
    event.clientY > box.bottom
  );
}

export function Modal({
  title,
  children,
  onClose,
  wide,
  className = "",
  closeDisabled = false,
}: {
  title: string;
  children: React.ReactNode;
  onClose: () => void;
  wide?: boolean;
  className?: string;
  closeDisabled?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const pressedOnBackdrop = useRef(false);
  useEffect(() => {
    const dialog = ref.current!;
    const previous = document.activeElement;
    dialog.showModal();
    return () => {
      dialog.close();
      if (previous instanceof HTMLElement && previous.isConnected)
        previous.focus();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      className={`modal ${wide ? "wide" : ""} ${className}`}
      aria-label={title}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onPointerDownCapture={(event) => {
        if (event.isPrimary)
          pressedOnBackdrop.current = event.button === 0 && isBackdrop(event);
      }}
      onPointerCancelCapture={(event) => {
        if (event.isPrimary) pressedOnBackdrop.current = false;
      }}
      onClick={(event) => {
        // A drag from the content can dispatch its click on the backdrop.
        const startedOnBackdrop = pressedOnBackdrop.current;
        pressedOnBackdrop.current = false;
        if (startedOnBackdrop && isBackdrop(event)) onClose();
      }}
    >
      <header>
        <h2>{title}</h2>
        <button
          className="icon"
          onClick={onClose}
          aria-label="关闭"
          disabled={closeDisabled}
        >
          <X size={19} />
        </button>
      </header>
      {React.useContext(ModalNotice)}
      {children}
    </dialog>
  );
}
