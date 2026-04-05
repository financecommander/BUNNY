const observer = new IntersectionObserver(
  (entries) => {
    entries.forEach((entry) => {
      if (entry.isIntersecting) {
        entry.target.classList.add("is-visible");
      }
    });
  },
  { threshold: 0.14 }
);

document
  .querySelectorAll(
    ".section-intro, .hero-proof, .hero-panel, .hero-band, .copy-block, .editorial-list article, .sector-grid article, .material-board, .process-flow article, .advantage-grid div, .faq-stack details, .send-checklist, .lead-form"
  )
  .forEach((el) => {
    el.classList.add("reveal");
    observer.observe(el);
  });

const navToggle = document.querySelector(".nav-toggle");
const siteNav = document.getElementById("site-nav");

if (navToggle && siteNav) {
  navToggle.addEventListener("click", () => {
    const isOpen = document.body.classList.toggle("nav-open");
    navToggle.setAttribute("aria-expanded", String(isOpen));
  });

  siteNav.querySelectorAll("a").forEach((link) => {
    link.addEventListener("click", () => {
      document.body.classList.remove("nav-open");
      navToggle.setAttribute("aria-expanded", "false");
    });
  });
}

const leadForm = document.getElementById("lead-form");
const formStatus = document.getElementById("form-status");

if (leadForm && formStatus) {
  leadForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    formStatus.textContent = "Sending request...";
    formStatus.className = "form-status";

    const formData = new FormData(leadForm);
    const payload = {
      full_name: formData.get("full_name"),
      work_email: formData.get("work_email"),
      company_name: formData.get("company_name"),
      phone_number: formData.get("phone_number") || null,
      use_case: formData.get("use_case"),
      part_type: formData.get("part_type"),
      timeline: formData.get("timeline"),
      quantity_estimate: formData.get("quantity_estimate")
        ? Number(formData.get("quantity_estimate"))
        : null,
      process_interests: String(formData.get("process_interests") || "")
        .split(",")
        .map((value) => value.trim())
        .filter(Boolean),
      material_interests: String(formData.get("material_interests") || "")
        .split(",")
        .map((value) => value.trim())
        .filter(Boolean),
      notes: formData.get("notes") || null,
      source: "website",
    };

    try {
      const intakeUrl = new URL(
        "manufacturing/intake-leads",
        `${window.location.href.replace(/[#?].*$/, "").replace(/[^/]*$/, "")}`
      );
      const response = await fetch(intakeUrl.toString(), {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify(payload),
      });

      if (!response.ok) {
        throw new Error(`request failed: ${response.status}`);
      }

      const result = await response.json();
      formStatus.textContent = `Request received. Lead id: ${result.manufacturing_intake_lead_id}. A Buildmesh operator will follow up by email.`;
      formStatus.className = "form-status is-success";
      leadForm.reset();
    } catch (error) {
      formStatus.textContent = "We couldn't submit the request automatically. Please try again shortly.";
      formStatus.className = "form-status is-error";
    }
  });
}
