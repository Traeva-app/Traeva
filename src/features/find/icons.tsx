// Find's icons (#198), drawn here: a page with a coloured band and the
// kind's label (an image's page also has a small picture), a folder, and
// the search box's magnifier. Their colours are
// in find.css. Find and the quick search window both use them.

import type { FileIcon } from "./view";

export function FileIconSvg({ icon }: { icon: FileIcon }) {
  const size = icon.label.length <= 1 ? 9 : icon.label.length >= 4 ? 6 : 7.5;
  return (
    <svg
      className={`find-icon find-icon-${icon.kind}`}
      width="30"
      height="34"
      viewBox="0 0 28 32"
      aria-hidden="true"
      focusable="false"
    >
      <path className="find-icon-page" d="M7 1.5h10.5l6 6v21a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2v-25a2 2 0 0 1 2-2z" />
      <path className="find-icon-fold" d="M17.5 1.5v4a2 2 0 0 0 2 2h4" />
      {icon.kind === "image" && (
        <>
          <rect className="find-icon-picture" x="7.5" y="4.5" width="9" height="8" rx="1" />
          <path className="find-icon-picture-fill" d="M8.5 11.5l2.4-3 1.6 1.9 1.1-1.2 1.9 2.3z" />
          <circle className="find-icon-picture-fill" cx="14.2" cy="6.8" r="0.9" />
        </>
      )}
      {icon.label !== "" && (
        <>
          <rect className="find-icon-band" x="1.5" y="15" width="19" height="10" rx="2" />
          <text
            className="find-icon-label"
            x="11"
            y="20.4"
            fontSize={size}
            textAnchor="middle"
            dominantBaseline="central"
          >
            {icon.label}
          </text>
        </>
      )}
    </svg>
  );
}

export function FolderIconSvg() {
  return (
    <svg className="find-icon" width="30" height="34" viewBox="0 0 28 32" aria-hidden="true" focusable="false">
      <path
        className="find-folder-back"
        d="M2 9.5a2 2 0 0 1 2-2h6.2l2.3 2.5H24a2 2 0 0 1 2 2V25a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2z"
      />
      <path className="find-folder-front" d="M2 13.5h24V25a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2z" />
    </svg>
  );
}

export function SearchGlyph() {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.9"
      strokeLinecap="round"
      aria-hidden="true"
      focusable="false"
    >
      <circle cx="11" cy="11" r="6.5" />
      <path d="m16 16 4.5 4.5" />
    </svg>
  );
}
