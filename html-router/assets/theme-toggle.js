// Global media query and listener state
const systemMediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
let isSystemListenerAttached = false;

const handleSystemThemeChange = (e) => {
    const themePreference = document.documentElement.getAttribute('data-theme-preference');
    if (themePreference === 'system') {
        document.documentElement.setAttribute('data-theme', e.matches ? 'dark' : 'light');
    }
    // For explicit themes like 'obsidian-prism', 'light', 'dark' - do nothing on system change
};

const initializeTheme = () => {
    const themeToggle = document.querySelector('.theme-controller');
    const themePreference = document.documentElement.getAttribute('data-theme-preference');

    if (themeToggle) {
        // Anonymous mode
        if (isSystemListenerAttached) {
            systemMediaQuery.removeEventListener('change', handleSystemThemeChange);
            isSystemListenerAttached = false;
        }

        // Avoid re-binding if already bound
        if (themeToggle.dataset.bound) return;
        themeToggle.dataset.bound = "true";

        // Detect system preference
        const prefersDark = systemMediaQuery.matches;

        // Initialize theme from local storage or system preference
        const savedTheme = localStorage.getItem('theme');
        const initialTheme = savedTheme ? savedTheme : (prefersDark ? 'dark' : 'light');
        document.documentElement.setAttribute('data-theme', initialTheme);
        themeToggle.checked = initialTheme === 'dark';

        // Update theme and local storage on toggle
        themeToggle.addEventListener('change', () => {
            const theme = themeToggle.checked ? 'dark' : 'light';
            document.documentElement.setAttribute('data-theme', theme);
            localStorage.setItem('theme', theme);
        });

    } else {
        // Authenticated mode
        localStorage.removeItem('theme');

        if (themePreference === 'system') {
            // Ensure correct theme is set immediately
            const currentSystemTheme = systemMediaQuery.matches ? 'dark' : 'light';
            // Only update if needed
            if (document.documentElement.getAttribute('data-theme') !== currentSystemTheme) {
                document.documentElement.setAttribute('data-theme', currentSystemTheme);
            }

            if (!isSystemListenerAttached) {
                systemMediaQuery.addEventListener('change', handleSystemThemeChange);
                isSystemListenerAttached = true;
            }
        } else {
            // Explicit theme: 'light', 'dark', 'obsidian-prism', etc.
            if (isSystemListenerAttached) {
                systemMediaQuery.removeEventListener('change', handleSystemThemeChange);
                isSystemListenerAttached = false;
            }
            // Ensure data-theme matches preference exactly
            if (themePreference && document.documentElement.getAttribute('data-theme') !== themePreference) {
                document.documentElement.setAttribute('data-theme', themePreference);
            }
        }
    }
};

// Run the initialization after the DOM is fully loaded
document.addEventListener('DOMContentLoaded', initializeTheme);

// Reinitialize theme toggle after HTMX swaps
document.addEventListener('htmx:afterSwap', initializeTheme);
document.addEventListener('htmx:afterSettle', initializeTheme);
