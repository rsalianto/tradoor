/**
 * A number typed in, landing as it is typed so what hangs on it follows at once.
 * What is typed stands while the field has the focus: 80. is on its way to 80.5,
 * and writing the value back would take the dot away. On leaving it the field
 * shows the value as it now is.
 */
import { createEffect } from 'solid-js';

export function NumField(props: { value: number; onValue(v: number): void; class?: string; step?: number | string; label?: string }) {
  let el!: HTMLInputElement;
  const show = () => (el.value = Number.isFinite(props.value) ? String(props.value) : '');
  createEffect(() => {
    void props.value;
    if (document.activeElement !== el) show();
  });
  return (
    <input
      ref={el}
      class={props.class ?? 'dinput num'}
      type="number"
      inputmode="decimal"
      step={props.step ?? 'any'}
      aria-label={props.label}
      // Focusing a number selects it, so what is typed replaces it.
      onFocus={(e) => e.currentTarget.select()}
      onInput={(e) => {
        const v = Number(e.currentTarget.value);
        if (e.currentTarget.value !== '' && Number.isFinite(v)) props.onValue(v);
      }}
      onBlur={show}
    />
  );
}
