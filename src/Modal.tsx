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

export function Modal({
  title,
  children,
  onClose,
  wide,
}: {
  title: string;
  children: React.ReactNode;
  onClose: () => void;
  wide?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
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
      className={`modal ${wide ? "wide" : ""}`}
      aria-label={title}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        if (event.target !== event.currentTarget) return;
        const box = event.currentTarget.getBoundingClientRect();
        if (
          event.clientX < box.left ||
          event.clientX > box.right ||
          event.clientY < box.top ||
          event.clientY > box.bottom
        )
          onClose();
      }}
    >
      <header>
        <h2>{title}</h2>
        <button className="icon" onClick={onClose} aria-label="关闭">
          <X size={19} />
        </button>
      </header>
      {React.useContext(ModalNotice)}
      {children}
    </dialog>
  );
}
