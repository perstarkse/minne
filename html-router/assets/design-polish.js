/**
 * Design Polishing Pass - Interactive Effects
 *
 * Includes:
 * - Scroll-Linked Navbar Shadow
 * - HTMX Swap Animation
 * - Typewriter AI Response
 * - Rubberbanding Scroll
 */

(() => {
	// === SCROLL-LINKED NAVBAR SHADOW ===
	function initScrollShadow() {
		const mainContent = document.querySelector("main");
		const navbar = document.querySelector("nav");
		if (!mainContent || !navbar) return;

		mainContent.addEventListener(
			"scroll",
			() => {
				const scrollTop = mainContent.scrollTop;
				const scrollHeight =
					mainContent.scrollHeight - mainContent.clientHeight;
				const scrollDepth = scrollHeight > 0 ? Math.min(scrollTop / 200, 1) : 0;
				navbar.style.setProperty("--scroll-depth", scrollDepth.toFixed(2));
			},
			{ passive: true },
		);
	}

	// === HTMX SWAP ANIMATION ===
	function initHtmxSwapAnimation() {
		document.body.addEventListener("htmx:afterSwap", (event) => {
			let target = event.detail.target;
			if (!target) return;

			// If full body swap (hx-boost), animate only the main content
			if (target.tagName === "BODY") {
				const main = document.querySelector("main");
				if (main) target = main;
			}

			// Only animate if target is valid and inside/is main content or a card/panel
			// Avoid animating sidebar or navbar updates
			if (target && (target.tagName === "MAIN" || target.closest("main"))) {
				if (!target.classList.contains("animate-fade-up")) {
					target.classList.add("animate-fade-up");
					// Remove class after animation completes to allow re-animation
					setTimeout(() => {
						target.classList.remove("animate-fade-up");
					}, 250);
				}
			}
		});
	}

	// === TYPEWRITER AI RESPONSE ===
	// Works with SSE streaming - buffers text and reveals character by character
	window.initTypewriter = (element, options = {}) => {
		const { minDelay = 5, maxDelay = 15, showCursor = true } = options;

		let buffer = "";
		let isTyping = false;
		let cursorElement = null;

		if (showCursor) {
			cursorElement = document.createElement("span");
			cursorElement.className = "typewriter-cursor";
			cursorElement.textContent = "▌";
			cursorElement.style.animation = "blink 1s step-end infinite";
			element.appendChild(cursorElement);
		}

		function typeNextChar() {
			if (buffer.length === 0) {
				isTyping = false;
				return;
			}

			isTyping = true;
			const char = buffer.charAt(0);
			buffer = buffer.slice(1);

			// Insert before cursor
			if (cursorElement && cursorElement.parentNode) {
				const textNode = document.createTextNode(char);
				element.insertBefore(textNode, cursorElement);
			} else {
				element.textContent += char;
			}

			const delay = minDelay + Math.random() * (maxDelay - minDelay);
			setTimeout(typeNextChar, delay);
		}

		return {
			append: (text) => {
				buffer += text;
				if (!isTyping) {
					typeNextChar();
				}
			},
			complete: () => {
				// Flush remaining buffer immediately
				if (cursorElement && cursorElement.parentNode) {
					const textNode = document.createTextNode(buffer);
					element.insertBefore(textNode, cursorElement);
					cursorElement.remove();
				} else {
					element.textContent += buffer;
				}
				buffer = "";
				isTyping = false;
			},
		};
	};

	// === RUBBERBANDING SCROLL ===
	function attachRubberbanding(
		container,
		{ maxPull = 60, resistance = 0.4 } = {},
	) {
		let startY = 0;
		let pulling = false;

		function applyPull(distance) {
			container.style.transform = `translateY(${distance}px)`;
		}

		function release() {
			container.style.transition =
				"transform 300ms cubic-bezier(0.25, 1, 0.5, 1)";
			container.style.transform = "translateY(0)";
			setTimeout(() => {
				container.style.transition = "";
			}, 300);
			pulling = false;
		}

		function isAtTop() {
			return container.scrollTop <= 0;
		}
		function isAtBottom() {
			return (
				container.scrollTop + container.clientHeight >= container.scrollHeight
			);
		}

		container.addEventListener(
			"touchstart",
			(e) => {
				startY = e.touches[0].clientY;
			},
			{ passive: true },
		);

		container.addEventListener(
			"touchmove",
			(e) => {
				const diff = e.touches[0].clientY - startY;
				const isPullingDown = diff > 0 && isAtTop();
				const isPullingUp = diff < 0 && isAtBottom();

				if (isPullingDown) {
					pulling = true;
					applyPull(Math.min(diff * resistance, maxPull));
				} else if (isPullingUp) {
					pulling = true;
					applyPull(Math.max(diff * resistance, -maxPull));
				}
			},
			{ passive: true },
		);

		container.addEventListener(
			"touchend",
			() => {
				if (pulling) release();
			},
			{ passive: true },
		);
	}

	function initRubberbanding() {
		document
			.querySelectorAll("#chat-scroll-container, .content-scroll-container")
			.forEach((container) => attachRubberbanding(container));
	}

	// === INITIALIZATION ===
	function init() {
		initScrollShadow();
		initHtmxSwapAnimation();
		initRubberbanding();
	}

	// Run on DOMContentLoaded
	if (document.readyState === "loading") {
		document.addEventListener("DOMContentLoaded", init);
	} else {
		init();
	}

	// Re-init rubberbanding after HTMX navigations
	document.body.addEventListener("htmx:afterSettle", () => {
		initRubberbanding();
	});

	// Add typewriter cursor blink animation
	const style = document.createElement("style");
	style.textContent = `
    @keyframes blink {
      0%, 100% { opacity: 1; }
      50% { opacity: 0; }
    }
    .typewriter-cursor {
      color: var(--color-accent);
      font-weight: bold;
    }
  `;
	document.head.appendChild(style);
})();
