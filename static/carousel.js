const IMG_LOAD_TIMEOUT_MS = 10000;
const imageTimers = new WeakMap();

function clearImageTimer(img) {
  const timer = imageTimers.get(img);
  if (timer) {
    clearTimeout(timer);
    imageTimers.delete(img);
  }
}

function startImageTimer(img, carousel) {
  clearImageTimer(img);
  const timer = setTimeout(() => {
    imageTimers.delete(img);
    if (img.complete && img.naturalWidth > 0) return;
    img.dataset.failed = "1";
    img.classList.remove("is-loaded");
    // Cancel pending stalled image request
    img.removeAttribute("src");
    if (carousel.querySelector(".carousel-slide.active") === img) {
      carousel.classList.remove("is-loading");
      carousel.classList.add("has-error");
    }
  }, IMG_LOAD_TIMEOUT_MS);
  imageTimers.set(img, timer);
}

function bindImageListeners(img, carousel) {
  if (img.dataset.imgBound === "1") return;
  img.dataset.imgBound = "1";
  if (!img.dataset.origSrc) {
    img.dataset.origSrc = img.getAttribute("src") || "";
  }

  img.addEventListener("load", () => {
    clearImageTimer(img);
    img.dataset.failed = "0";
    img.classList.add("is-loaded");
    if (carousel.querySelector(".carousel-slide.active") === img) {
      carousel.classList.remove("is-loading", "has-error");
    }
  });

  img.addEventListener("error", () => {
    clearImageTimer(img);
    img.dataset.failed = "1";
    img.classList.remove("is-loaded");
    if (carousel.querySelector(".carousel-slide.active") === img) {
      carousel.classList.remove("is-loading");
      carousel.classList.add("has-error");
    }
  });
}

function syncCarouselLoading(carousel) {
  carousel.querySelectorAll(".carousel-slide").forEach((img) => {
    bindImageListeners(img, carousel);
  });

  const activeImg = carousel.querySelector(".carousel-slide.active");
  if (!activeImg) {
    carousel.classList.remove("is-loading", "has-error");
    return;
  }

  if (activeImg.complete && activeImg.naturalWidth > 0) {
    clearImageTimer(activeImg);
    activeImg.dataset.failed = "0";
    carousel.classList.remove("is-loading", "has-error");
    activeImg.classList.add("is-loaded");
    return;
  }

  if (activeImg.dataset.failed === "1" || (activeImg.complete && activeImg.naturalWidth === 0)) {
    clearImageTimer(activeImg);
    activeImg.dataset.failed = "1";
    carousel.classList.remove("is-loading");
    carousel.classList.add("has-error");
    return;
  }

  carousel.classList.remove("has-error");
  carousel.classList.add("is-loading");
  activeImg.classList.remove("is-loaded");
  startImageTimer(activeImg, carousel);
}

function initAllCarousels(root = document) {
  root.querySelectorAll(".card-carousel").forEach(syncCarouselLoading);
}

function setCarouselSlide(carousel, index) {
  const slides = carousel.querySelectorAll(".carousel-slide");
  const dots = carousel.querySelectorAll(".carousel-dot");
  if (!slides.length) return;

  const total = slides.length;
  const next = ((index % total) + total) % total;

  slides.forEach((slide, i) => {
    slide.classList.toggle("active", i === next);
  });
  dots.forEach((btn, i) => {
    btn.classList.toggle("active", i === next);
    btn.setAttribute("aria-selected", i === next ? "true" : "false");
  });

  syncCarouselLoading(carousel);
}

document.addEventListener("DOMContentLoaded", () => initAllCarousels(document));
document.addEventListener("htmx:afterSwap", (event) => {
  initAllCarousels(event.target || document);
});

document.addEventListener("click", (event) => {
  // Manual refetch button when an image fails or times out
  const retryBtn = event.target.closest(".img-retry-btn");
  if (retryBtn) {
    event.preventDefault();
    event.stopPropagation();

    const carousel = retryBtn.closest(".card-carousel");
    const activeImg = carousel?.querySelector(".carousel-slide.active");
    if (!carousel || !activeImg) return;

    const orig = activeImg.dataset.origSrc || activeImg.getAttribute("src") || "";
    if (!orig) return;

    activeImg.dataset.failed = "0";
    carousel.classList.remove("has-error");
    carousel.classList.add("is-loading");
    activeImg.classList.remove("is-loaded");
    startImageTimer(activeImg, carousel);

    const sep = orig.includes("?") ? "&" : "?";
    activeImg.src = `${orig}${sep}t=${Date.now()}`;
    return;
  }

  const dot = event.target.closest(".carousel-dot");
  if (!dot) return;

  event.preventDefault();
  event.stopPropagation();

  const carousel = dot.closest(".card-carousel");
  if (!carousel) return;

  setCarouselSlide(carousel, Number(dot.dataset.index));
});

// Touch swipe support for mobile carousels
let touchStartX = 0;
let touchStartY = 0;
let activeCarousel = null;

document.addEventListener(
  "touchstart",
  (event) => {
    const carousel = event.target.closest(".card-carousel");
    if (!carousel || event.touches.length !== 1) return;
    activeCarousel = carousel;
    touchStartX = event.touches[0].clientX;
    touchStartY = event.touches[0].clientY;
  },
  { passive: true }
);

document.addEventListener(
  "touchend",
  (event) => {
    if (!activeCarousel || !event.changedTouches.length) return;
    const dx = event.changedTouches[0].clientX - touchStartX;
    const dy = event.changedTouches[0].clientY - touchStartY;
    const carousel = activeCarousel;
    activeCarousel = null;

    if (Math.abs(dx) < 35 || Math.abs(dx) < Math.abs(dy)) return;

    const activeDot = carousel.querySelector(".carousel-dot.active");
    const currentIndex = activeDot ? Number(activeDot.dataset.index) : 0;
    setCarouselSlide(carousel, dx < 0 ? currentIndex + 1 : currentIndex - 1);
  },
  { passive: true }
);
