
  <!-- Theme switch script -->
  <script>
    const initializeTheme = () => {
      console.log("Initializing theme toggle...");
      const themeToggle = document.querySelector('.theme-controller');
      if (!themeToggle) {
        console.log("Theme toggle not found.");
        return;
      }

      // Detect system preference
      const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;

      // Initialize theme from local storage or system preference
      const savedTheme = localStorage.getItem('theme');
      const initialTheme = savedTheme ? savedTheme : (prefersDark ? 'dark' : 'light');
      document.documentElement.setAttribute('data-theme', initialTheme);
      themeToggle.checked = initialTheme === 'dark';

      // Update theme and local storage on toggle
      themeToggle.addEventListener('change', () => {
        const theme = themeToggle.checked ? 'dark' : 'light';
        console.log("Theme switched to:", theme);
        document.documentElement.setAttribute('data-theme', theme);
        localStorage.setItem('theme', theme);
      });

      console.log("Theme toggle initialized.");
    };

    // Run the initialization after the DOM is fully loaded
    document.addEventListener('DOMContentLoaded', () => {
      console.log("DOM fully loaded. Initializing theme toggle...");
      initializeTheme();
    });

    // Reinitialize theme toggle after HTMX swaps
    document.addEventListener('htmx:afterSwap', initializeTheme);
    document.addEventListener('htmx:afterSettle', initializeTheme);
  </script>
