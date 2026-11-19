export const EASE_OUT = [0.23, 1, 0.32, 1] as const;
export const EASE_IN_OUT = [0.77, 0, 0.175, 1] as const;

/** CSS versions of the curves, for transitions and inline styles. */
export const CSS_EASE_OUT = "cubic-bezier(0.23, 1, 0.32, 1)";
export const CSS_EASE_IN_OUT = "cubic-bezier(0.77, 0, 0.175, 1)";

export const spring = { type: "spring", duration: 0.3, bounce: 0 } as const;

export const enter = { duration: 0.2, ease: EASE_OUT } as const;
export const exit = { duration: 0.14, ease: EASE_OUT } as const;

/** Instant transition for keyboard-initiated changes and reduced motion. */
export const instant = { duration: 0 } as const;

/*
 * Movimento próprio do Dita.se (docs/motion.md). O que o Tuff UI já anima (gráficos, números,
 * menus, toasts, diálogos, botões) fica com ele; aqui só entram a casca e as listas.
 */

/** Página nova: o conteúdo sobe 6 px enquanto aparece. A página antiga some na hora, sem esperar. */
export const pagina = { duration: 0.22, ease: EASE_OUT } as const;

/** Indicador da navegação lateral deslizando de um item para outro. */
export const indicador = { type: "spring", duration: 0.34, bounce: 0 } as const;

/** Linha de lista que entra (item novo) ou sai (apagado): altura e opacidade. */
export const linha = { duration: 0.2, ease: EASE_OUT } as const;

/** Folga entre linhas das listas (gap-1.5 = 6 px), para a linha recolhida não deixar espaço. */
export const FOLGA_DA_LISTA = -6;
