import { useLayoutEffect, useRef, useState, type ClipboardEvent, type DragEvent, type ReactNode } from 'react';
import { useHost } from '@cortex/sdk';
import {
  MAX_ATTACHMENTS,
  PASTE_AS_FILE,
  attachOne,
  baseName,
  fromDataTransfer,
  humanSize,
  isLongPaste,
  pastedPaths,
  type AttachContext,
  type Attachment,
  type Incoming,
} from './attachments';

/**
 * Where a request is written: CortexCode's composer, without its modes, targets and mentions. One
 * box, the field, then its controls inside it: + on the left, Send on the right. Enter sends.
 *
 * Attachments: paste an image or files copied in Finder, drop them on the box, or pick them with +.
 * Each is a chip (a preview for images) with × to take it off. Pasted text over the paste-as-file
 * limit becomes a text file ("Pasted text · N lines"); View shows it, Inline puts it back.
 */
export function Composer({
  draft,
  onDraft,
  onSend,
  disabled,
  placeholder,
  attachments,
  onAttachments,
  attach,
  sendLabel = 'Send',
  autoFocus,
  left,
  right,
}: {
  draft: string;
  onDraft: (text: string) => void;
  onSend: (text: string, attachments: Attachment[]) => Promise<boolean>;
  disabled: boolean;
  placeholder: string;
  attachments: Attachment[];
  onAttachments: (update: (current: Attachment[]) => Attachment[]) => void;
  attach: AttachContext;
  sendLabel?: string;
  autoFocus?: boolean;
  /** Controls after + (a switch, a working line). */
  left?: ReactNode;
  /** Controls before Send (Stop). */
  right?: ReactNode;
}) {
  const [sending, setSending] = useState(false);
  const area = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const [making, setMaking] = useState<Array<{ id: number; name: string }>>([]);
  const [problems, setProblems] = useState<string[]>([]);
  const [dragging, setDragging] = useState(false);
  const nextId = useRef(0);
  const pastedTexts = useRef(new Map<string, string>());
  const [viewing, setViewing] = useState<string | null>(null);

  const add = (incoming: Incoming[]) => {
    if (incoming.length === 0) return;
    const room = MAX_ATTACHMENTS - attachments.length - making.length;
    const accepted = incoming.slice(0, Math.max(0, room));
    const refused = incoming.length - accepted.length;
    if (refused > 0) {
      setProblems((current) => [...current, `${refused === 1 ? 'One item was' : `${refused} items were`} not attached: a message may carry at most ${MAX_ATTACHMENTS} attachments.`]);
    }
    for (const item of accepted) {
      const id = nextId.current++;
      const name = item.type === 'file' ? item.file.name || 'pasted image' : item.name;
      setMaking((current) => [...current, { id, name: item.type === 'text' ? `${name} (pasted text)` : name }]);
      attachOne(item, attach)
        .then((made) => {
          if (item.type === 'text') pastedTexts.current.set(made.path, item.text);
          onAttachments((current) => [...current, made]);
        })
        .catch((cause: unknown) => setProblems((current) => [...current, cause instanceof Error ? cause.message : String(cause)]))
        .finally(() => setMaking((current) => current.filter((entry) => entry.id !== id)));
    }
  };

  const onPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const data = event.clipboardData;
    const files = data ? Array.from(data.files ?? []) : [];
    const paths = files.length === 0 && data ? pastedPaths(data) : [];
    if (paths.length > 0) {
      event.preventDefault();
      add(paths);
      return;
    }
    if (files.length === 0) {
      const text = data?.getData('text/plain') ?? '';
      if (!isLongPaste(text, PASTE_AS_FILE)) return;
      event.preventDefault();
      const number = attachments.filter((attachment) => attachment.pasted_text).length + making.length + 1;
      add([{ type: 'text', text, name: `pasted-${number}.txt` }]);
      return;
    }
    const types = data ? Array.from(data.types ?? []) : [];
    if (!types.includes('text/plain')) event.preventDefault();
    add(files.map((file) => ({ type: 'file', file, pasted: true })));
  };

  const carriesFiles = (event: DragEvent) => Array.from(event.dataTransfer?.types ?? []).includes('Files');
  const onDragOver = (event: DragEvent<HTMLDivElement>) => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy';
    if (!dragging) setDragging(true);
  };
  const onDrop = (event: DragEvent<HTMLDivElement>) => {
    setDragging(false);
    if (!event.dataTransfer || !carriesFiles(event)) return;
    event.preventDefault();
    add(fromDataTransfer(event.dataTransfer, attach.shell));
  };

  const pick = async () => {
    if (!attach.shell?.pickPaths) {
      picker.current?.click();
      return;
    }
    const paths = await attach.shell.pickPaths('Attach files or folders');
    add(paths.map((path) => ({ type: 'path', name: baseName(path), path })));
  };

  const remove = (index: number) => onAttachments((current) => current.filter((_, at) => at !== index));

  const inline = (index: number) => {
    const attachment = attachments[index];
    const text = attachment ? pastedTexts.current.get(attachment.path) : undefined;
    if (!attachment || text === undefined) return;
    const element = area.current;
    const at = element ? element.selectionStart ?? draft.length : draft.length;
    onDraft(draft.slice(0, at) + text + draft.slice(at));
    remove(index);
    if (viewing === attachment.path) setViewing(null);
  };

  // The field grows with what is written, up to the design's limit (`.write` max-height).
  useLayoutEffect(() => {
    const element = area.current;
    if (!element) return;
    element.style.height = 'auto';
    element.style.height = `${element.scrollHeight + 2}px`;
  }, [draft]);

  const ready = making.length === 0;
  const canSend = (draft.trim() !== '' || attachments.length > 0) && ready && !sending && !disabled;
  const send = async () => {
    if (!canSend) return;
    setSending(true);
    try {
      if (await onSend(draft.trim(), attachments)) {
        onDraft('');
        onAttachments(() => []);
        setProblems([]);
      }
    } finally {
      setSending(false);
      area.current?.focus({ preventScroll: true });
    }
  };

  return (
    <div className="cx-composer" data-dragging={dragging ? 'true' : undefined}
      onDragOver={onDragOver} onDragLeave={() => setDragging(false)} onDrop={onDrop}>
      {(attachments.length > 0 || making.length > 0) && (
        <ul className="attached cx-attached" aria-label="Attachments">
          {attachments.map((attachment, index) => attachment.pasted_text ? (
            <li key={`${attachment.path}:${index}`} className="chip cx-chip" data-kind="pasted-text" title={attachment.name}>
              <span className="cx-chip-name">Pasted text · {(attachment.lines ?? 0).toLocaleString('en-US')} lines</span>
              <span className="cx-chip-size">{humanSize(attachment.bytes)}</span>
              {pastedTexts.current.has(attachment.path) && (
                <>
                  <button type="button" aria-expanded={viewing === attachment.path} aria-label={`View ${attachment.name}`}
                    onClick={() => setViewing(viewing === attachment.path ? null : attachment.path)}>View</button>
                  <button type="button" aria-label={`Put ${attachment.name} back into the message`} onClick={() => inline(index)}>Inline</button>
                </>
              )}
              <button type="button" aria-label={`Remove ${attachment.name}`} onClick={() => remove(index)}>×</button>
            </li>
          ) : (
            <li key={`${attachment.path}:${index}`} className="chip cx-chip" data-kind={attachment.kind} title={attachment.name}>
              {attachment.thumb && <img className="cx-thumb" src={attachment.thumb} alt="" />}
              <span className="cx-chip-kind">{attachment.kind}</span>
              <span className="cx-chip-name">{attachment.name}</span>
              {attachment.kind !== 'folder' && <span className="cx-chip-size">{humanSize(attachment.bytes)}</span>}
              <button type="button" aria-label={`Remove ${attachment.name}`} onClick={() => remove(index)}>×</button>
            </li>
          ))}
          {making.map((entry) => (
            <li key={`making-${entry.id}`} className="chip cx-chip" data-making="true" aria-busy="true">
              <span className="cx-chip-name">{entry.name}</span>
              <span className="cx-chip-size">attaching…</span>
            </li>
          ))}
        </ul>
      )}
      {viewing !== null && pastedTexts.current.has(viewing) && (
        <pre className="run-output cx-pasted-view" aria-label="Pasted text">{pastedTexts.current.get(viewing)}</pre>
      )}
      {problems.length > 0 && (
        <div className="cx-attach-problems" role="alert">
          {problems.map((problem, index) => <p key={index} className="inline-error">{problem}</p>)}
          <button className="detent" type="button" onClick={() => setProblems([])}>Dismiss</button>
        </div>
      )}
      <div className="cx-box">
        <textarea
          ref={area}
          className="write"
          rows={1}
          value={draft}
          disabled={disabled}
          autoFocus={autoFocus}
          placeholder={placeholder}
          aria-label="Message"
          onChange={(event) => onDraft(event.target.value)}
          onPaste={onPaste}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void send();
              return;
            }
            if (event.key === 'Backspace' && draft === '' && attachments.length > 0) {
              event.preventDefault();
              remove(attachments.length - 1);
            }
          }}
        />
        <input ref={picker} type="file" multiple hidden aria-hidden="true" tabIndex={-1}
          onChange={(event) => {
            const files = Array.from(event.currentTarget.files ?? []);
            event.currentTarget.value = '';
            add(files.map((file) => ({ type: 'file', file, pasted: false })));
          }} />
        <div className="tools cx-tools" role="group" aria-label="Message tools">
          <button type="button" className="cx-plus" disabled={disabled} onClick={() => void pick()} aria-label="Attach"
            title="Attach files, images or folders">+</button>
          {left}
          <div className="grow" />
          {right}
          <button className="detent send" type="button" disabled={!canSend} onClick={() => void send()}>
            {sending ? 'Sending…' : sendLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

/** What was attached to a message in the thread: chips, a preview for images; a click shows it in Finder. */
export function Sent({ list }: { list: unknown }) {
  const host = useHost();
  if (!Array.isArray(list) || list.length === 0) return null;
  const items = list.filter((item): item is Attachment => Boolean(item) && typeof item === 'object' && typeof (item as Attachment).path === 'string');
  if (items.length === 0) return null;
  return (
    <ul className="attached cx-sent" aria-label="Attachments">
      {items.map((item, index) => (
        <li key={`${item.path}:${index}`} className="chip cx-chip" data-kind={item.kind}>
          <button type="button" className="cx-chip-open" disabled={!host.shell} title={item.name}
            onClick={() => void host.shell?.reveal(item.path)}>
            {typeof item.thumb === 'string' && item.thumb.startsWith('data:image/') && <img className="cx-thumb" src={item.thumb} alt="" />}
            {!item.pasted_text && <span className="cx-chip-kind">{item.kind}</span>}
            <span className="cx-chip-name">{item.pasted_text ? `Pasted text · ${(item.lines ?? 0).toLocaleString('en-US')} lines` : item.name}</span>
            {item.kind !== 'folder' && typeof item.bytes === 'number' && <span className="cx-chip-size">{humanSize(item.bytes)}</span>}
          </button>
        </li>
      ))}
    </ul>
  );
}
