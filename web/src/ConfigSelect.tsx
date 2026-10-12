import { getLocale } from "@xcss/web/admin-ui/i18n";
import {useEffect, useRef, useState, type KeyboardEvent} from "react";

type Option = {value: string; label: string};
type Props = {
  id: string;
  value: string;
  options: Option[];
  disabled: boolean;
  describedBy?: string;
  onChange(value: string): void;
};

export function ConfigSelect({id, value, options, disabled, describedBy, onChange}: Props) {
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const search = useRef({text: "", time: 0});
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [placement, setPlacement] = useState({above: false, height: 260});
  const selected = options.findIndex(option => option.value === value);
  const expanded = open && !disabled;

  useEffect(() => { if (disabled) setOpen(false); }, [disabled]);

  useEffect(() => {
    if (!expanded) return;
    function dismiss(event: PointerEvent) {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    }
    document.addEventListener("pointerdown", dismiss);
    return () => document.removeEventListener("pointerdown", dismiss);
  }, [expanded]);

  useEffect(() => {
    if (expanded) list.current?.querySelectorAll('[role="option"]')[active]?.scrollIntoView({block: "nearest"});
  }, [expanded, active]);

  function show(index = Math.max(0, selected)) {
    const bounds = trigger.current!.getBoundingClientRect();
    const below = window.innerHeight - bounds.bottom - 12;
    const above = bounds.top - 12;
    const upwards = below < Math.min(260, options.length * 44 + 12) && above > below;
    setPlacement({above: upwards, height: Math.max(44, Math.min(260, upwards ? above : below))});
    setActive(index);
    search.current = {text: "", time: 0};
    setOpen(true);
  }

  function choose(index: number) {
    onChange(options[index].value);
    setOpen(false);
    trigger.current?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    if (event.key === "Tab") { setOpen(false); return; }
    if (event.key === "Escape") {
      if (expanded) { event.preventDefault(); event.stopPropagation(); setOpen(false); }
      return;
    }
    if (["ArrowDown", "ArrowUp", "Home", "End", "Enter", " "].includes(event.key)) {
      event.preventDefault();
      if (event.key === "Enter" || event.key === " ") {
        if (expanded) choose(active); else show();
      } else if (event.key === "Home" || event.key === "End") {
        const index = event.key === "Home" ? 0 : options.length - 1;
        if (expanded) setActive(index); else show(index);
      } else if (!expanded) show();
      else setActive(index => Math.max(0, Math.min(options.length - 1, index + (event.key === "ArrowDown" ? 1 : -1))));
      return;
    }
    if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault();
      const text = (Date.now() - search.current.time < 700 ? search.current.text : "") + event.key.toLocaleLowerCase(getLocale());
      const index = options.findIndex(option => option.label.toLocaleLowerCase(getLocale()).startsWith(text));
      if (index >= 0) { if (expanded) setActive(index); else show(index); }
      search.current = {text, time: Date.now()};
    }
  }

  return <div className="sunshine-config-select" ref={root} onBlur={event => {
    if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
  }}>
    <button ref={trigger} id={id} type="button" className="sunshine-config-select-trigger"
      role="combobox" aria-labelledby={`${id}-label`} aria-describedby={describedBy}
      aria-haspopup="listbox" aria-expanded={expanded} aria-controls={expanded ? `${id}-options` : undefined}
      aria-activedescendant={expanded ? `${id}-option-${active}` : undefined} disabled={disabled}
      onClick={() => expanded ? setOpen(false) : show()} onKeyDown={onKeyDown}>
      <span>{options[selected]?.label ?? value}</span>
      <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"><path d="m9 6 6 6-6 6"/></svg>
    </button>
    {expanded && <ul ref={list} id={`${id}-options`} role="listbox" aria-labelledby={`${id}-label`}
      className="sunshine-config-select-options" data-above={placement.above} style={{maxHeight: placement.height}}>
      {options.map((option, index) => <li key={option.value} id={`${id}-option-${index}`} role="option"
        aria-selected={option.value === value} data-active={index === active}
        onPointerMove={() => setActive(index)} onMouseDown={event => event.preventDefault()} onClick={() => choose(index)}>
        <span>{option.label}</span><span aria-hidden="true">{option.value === value ? "✓" : ""}</span>
      </li>)}
    </ul>}
  </div>;
}
