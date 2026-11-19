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
}

document.addEventListener("click", (event) => {
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
