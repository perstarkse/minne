  document.addEventListener("DOMContentLoaded", function () {
    window.show_toast = function (description, type = 'info', title = null) {
      const container = document.getElementById('toast-container');
      if (!container) {
        console.error("Toast container not found!");
        return;
      }
      const alert = document.createElement('div');
      // Base classes for the alert
      alert.className = `alert alert-${type} mt-2 shadow-md flex flex-col text-start`;

      // Build inner HTML based on whether title is provided
      let innerHTML = '';
      if (title) {
        innerHTML += `<div class="font-bold text-lg">${title}</div>`; // Title element
        innerHTML += `<div>${description}</div>`; // Description element
      } else {
        // Structure without title
        innerHTML += `<span>${description}</span>`;
      }

      alert.innerHTML = innerHTML;
      container.appendChild(alert);

      // Auto-remove after a delay
      setTimeout(() => {
        // Optional: Add fade-out effect
        alert.style.opacity = '0';
        alert.style.transition = 'opacity 0.5s ease-out';
        setTimeout(() => alert.remove(), 500); // Remove after fade
      }, 3000); // Start fade-out after 3 seconds
    };

    document.body.addEventListener('toast', function (event) {
      console.log(event);
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

