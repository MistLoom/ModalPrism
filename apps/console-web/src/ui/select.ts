export type SelectOption = { value: string; label: string };

export function mountSelect(
  host: HTMLElement,
  options: SelectOption[],
  value: string,
  onChange: (value: string) => void,
  placeholder = "请选择",
) {
  const selected = options.find((o) => o.value === value);
  host.className = "custom-select";
  host.innerHTML = `
    <button type="button" class="custom-select-trigger" aria-haspopup="listbox" aria-expanded="false">
      <span class="custom-select-label">${selected?.label ?? placeholder}</span>
      <span class="custom-select-chevron" aria-hidden="true"></span>
    </button>
    <ul class="custom-select-menu" role="listbox" hidden></ul>
  `;
  const menu = host.querySelector(".custom-select-menu") as HTMLElement;
  const trigger = host.querySelector(".custom-select-trigger") as HTMLButtonElement;
  const label = host.querySelector(".custom-select-label") as HTMLElement;

  const renderMenu = () => {
    menu.innerHTML = options
      .map(
        (o) =>
          `<li role="option" data-value="${escapeAttr(o.value)}" aria-selected="${o.value === value}">${escapeHtml(o.label)}</li>`,
      )
      .join("");
  };
  renderMenu();

  const close = () => {
    menu.hidden = true;
    trigger.setAttribute("aria-expanded", "false");
  };

  trigger.addEventListener("click", (e) => {
    e.stopPropagation();
    const open = menu.hidden;
    document.querySelectorAll(".custom-select-menu").forEach((el) => {
      (el as HTMLElement).hidden = true;
    });
    menu.hidden = !open;
    trigger.setAttribute("aria-expanded", String(open));
  });

  menu.addEventListener("click", (e) => {
    const li = (e.target as HTMLElement).closest("[data-value]") as HTMLElement | null;
    if (!li) return;
    const next = li.dataset.value ?? "";
    value = next;
    label.textContent = options.find((o) => o.value === next)?.label ?? placeholder;
    renderMenu();
    close();
    onChange(next);
  });

  document.addEventListener("click", close);

  return {
    setValue(next: string) {
      value = next;
      label.textContent = options.find((o) => o.value === next)?.label ?? placeholder;
      renderMenu();
    },
    setOptions(next: SelectOption[]) {
      options = next;
      renderMenu();
    },
  };
}

function escapeHtml(s: string) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/"/g, "&quot;");
}

function escapeAttr(s: string) {
  return escapeHtml(s);
}
