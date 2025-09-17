  document.addEventListener("DOMContentLoaded", function () {
    window.show_toast = function (description, type = 'info', title = null) {
      const container = document.getElementById('toast-container');
      if (!container) {
        console.error("Toast container not found!");
        return;
      }
      const alert = document.createElement('div');
      alert.className = `alert toast-alert alert-${type}`;
      alert.style.opacity = '1';
      alert.style.transition = 'opacity 0.5s ease-out';

      if (title) {
        const titleEl = document.createElement('div');
        titleEl.className = 'toast-alert-title';
        titleEl.textContent = title;
        alert.appendChild(titleEl);
      }

      const bodyEl = document.createElement(title ? 'div' : 'span');
      bodyEl.textContent = description;
      alert.appendChild(bodyEl);

      container.appendChild(alert);

      // Auto-remove after a delay
      setTimeout(() => {
        alert.style.opacity = '0';
        setTimeout(() => alert.remove(), 500);
      }, 3000);
    };

    document.body.addEventListener('toast', function (event) {
      // Extract data from the event detail, matching the Rust payload
      const detail = event.detail;
      if (detail && detail.description) {
        const description = detail.description;
        const type = detail.type || 'info'; // Default to 'info'
        const title = detail.title || null;  // Get title, default to null if missing

        // Call the updated show_toast function
        window.show_toast(description, type, title);
      } else {
        console.warn("Received toast event without detail.description", detail);
        // Fallback toast if description is missing
        window.show_toast("An event occurred, but details are missing.", "warning");
      }
    });

    document.body.addEventListener('htmx:beforeRequest', function (evt) {
      const container = document.getElementById('toast-container');
      if (container) container.innerHTML = '';
    });
  })
