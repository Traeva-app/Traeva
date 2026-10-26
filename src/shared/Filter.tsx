// A small filter over a page's groups (#37, the student's choice 10/14):
// one choice at a time, like a segmented control, never the accent colour
// (that is the page's one main button). Arrow keys move between choices.
// A page that filters uses filter.ts so that something new that waits for
// the user is never kept out of sight.

import { useRef, type KeyboardEvent } from "react";
import "./groups.css";

export function Filter<K extends string>(props: {
  /** What it filters, for screen readers, e.g. "Show". */
  label: string;
  value: K;
  options: { key: K; label: string; count?: number; attention?: boolean }[];
  onChange: (key: K) => void;
}) {
  const { options, value } = props;
  const box = useRef<HTMLDivElement>(null);
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 0;
    if (step === 0) return;
    event.preventDefault();
    const at = options.findIndex((option) => option.key === value);
    const next = options[(at + step + options.length) % options.length];
    props.onChange(next.key);
    box.current?.querySelector<HTMLButtonElement>(`[data-key="${next.key}"]`)?.focus();
  };
  return (
    <div ref={box} className="filter" role="radiogroup" aria-label={props.label} onKeyDown={onKeyDown}>
      {options.map((option) => {
        const chosen = option.key === value;
        return (
          <button
            key={option.key}
            type="button"
            role="radio"
            aria-checked={chosen}
            tabIndex={chosen ? 0 : -1}
            data-key={option.key}
            className={chosen ? "filter-option filter-chosen" : "filter-option"}
            onClick={() => props.onChange(option.key)}
          >
            {option.label}
            {option.count !== undefined && (
              <span className={option.attention && option.count > 0 ? "filter-count filter-count-attention" : "filter-count"}>
                {option.count}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
