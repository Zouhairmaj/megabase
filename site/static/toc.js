(() => {
  const navs = document.querySelectorAll("[data-toc]");
  if (!navs.length) return;

  const links = [...navs].flatMap((nav) => [
    ...nav.querySelectorAll('a[href^="#"]'),
  ]);
  const sections = links
    .map((link) => document.querySelector(link.hash))
    .filter(Boolean);
  if (!sections.length) return;

  const setActive = (id) => {
    links.forEach((link) => {
      const on = link.hash === `#${id}`;
      link.classList.toggle("is-active", on);
      if (on) link.setAttribute("aria-current", "location");
      else link.removeAttribute("aria-current");
    });
  };

  const offsets = () =>
    sections
      .map((section) => ({
        id: section.id,
        top: section.getBoundingClientRect().top,
      }))
      .filter((row) => row.top <= window.innerHeight * 0.28)
      .sort((a, b) => b.top - a.top);

  const update = () => {
    const seen = offsets();
    if (seen[0]) setActive(seen[0].id);
    else setActive(sections[0].id);
  };

  links.forEach((link) => {
    link.addEventListener("click", () => {
      const details = link.closest("details");
      if (details) details.open = false;
    });
  });

  window.addEventListener("scroll", update, { passive: true });
  window.addEventListener("resize", update);
  update();
})();
