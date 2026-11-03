// Populate the sidebar
//
// This is a script, and not included directly in the page, to control the total size of the book.
// The TOC contains an entry for each page, so if each page includes a copy of the TOC,
// the total size of the page becomes O(n**2).
class MDBookSidebarScrollbox extends HTMLElement {
    constructor() {
        super();
    }
    connectedCallback() {
        this.innerHTML = '<ol class="chapter"><li class="chapter-item expanded affix "><a href="zngur.html">Zngur</a></li><li class="chapter-item expanded "><a href="tutorial.html"><strong aria-hidden="true">1.</strong> Tutorial</a></li><li class="chapter-item expanded "><a href="call_rust_from_cpp/index.html"><strong aria-hidden="true">2.</strong> Calling Rust from C++</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="call_rust_from_cpp/name_mapping.html"><strong aria-hidden="true">2.1.</strong> Name mapping</a></li><li class="chapter-item expanded "><a href="call_rust_from_cpp/wellknown_traits.html"><strong aria-hidden="true">2.2.</strong> Well-known traits</a></li><li class="chapter-item expanded "><a href="call_rust_from_cpp/layout_policy.html"><strong aria-hidden="true">2.3.</strong> Layout policy</a></li><li class="chapter-item expanded "><a href="call_rust_from_cpp/fields.html"><strong aria-hidden="true">2.4.</strong> Fields</a></li><li class="chapter-item expanded "><a href="call_rust_from_cpp/variants.html"><strong aria-hidden="true">2.5.</strong> Variants</a></li><li class="chapter-item expanded "><a href="call_rust_from_cpp/special_types.html"><strong aria-hidden="true">2.6.</strong> Types with special support</a></li><li class="chapter-item expanded "><a href="call_rust_from_cpp/coroutines.html"><strong aria-hidden="true">2.7.</strong> Coroutines and Future</a></li><li class="chapter-item expanded "><a href="call_rust_from_cpp/panic_and_exceptions.html"><strong aria-hidden="true">2.8.</strong> Panic and exceptions</a></li><li class="chapter-item expanded "><a href="call_rust_from_cpp/raw_pointers.html"><strong aria-hidden="true">2.9.</strong> Raw pointers</a></li><li class="chapter-item expanded "><a href="template_types.html"><strong aria-hidden="true">2.10.</strong> Template types</a></li></ol></li><li class="chapter-item expanded "><a href="call_cpp_from_rust/index.html"><strong aria-hidden="true">3.</strong> Calling C++ from Rust</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="call_cpp_from_rust/function.html"><strong aria-hidden="true">3.1.</strong> Calling C++ free functions</a></li><li class="chapter-item expanded "><a href="call_cpp_from_rust/rust_impl.html"><strong aria-hidden="true">3.2.</strong> Writing impl blocks for Rust types in C++</a></li><li class="chapter-item expanded "><div><strong aria-hidden="true">3.3.</strong> Box&lt;dyn Fn&gt;</div></li><li class="chapter-item expanded "><a href="call_cpp_from_rust/opaque.html"><strong aria-hidden="true">3.4.</strong> Opaque C++ types</a></li><li class="chapter-item expanded "><a href="call_cpp_from_rust/trait_object.html"><strong aria-hidden="true">3.5.</strong> Trait objects</a></li></ol></li><li class="chapter-item expanded "><a href="import.html"><strong aria-hidden="true">4.</strong> Import</a></li><li class="chapter-item expanded "><a href="conditional_compilation.html"><strong aria-hidden="true">5.</strong> Conditional Compilation</a></li><li class="chapter-item expanded "><a href="safety.html"><strong aria-hidden="true">6.</strong> Safety</a></li><li class="chapter-item expanded "><a href="how_it_compares.html"><strong aria-hidden="true">7.</strong> How it compares to other tools</a></li><li class="chapter-item expanded "><a href="philosophy.html"><strong aria-hidden="true">8.</strong> Design decisions</a></li><li class="chapter-item expanded "><a href="how_it_works.html"><strong aria-hidden="true">9.</strong> How it works</a></li></ol>';
        // Set the current, active page, and reveal it if it's hidden
        let current_page = document.location.href.toString().split("#")[0];
        if (current_page.endsWith("/")) {
            current_page += "index.html";
        }
        var links = Array.prototype.slice.call(this.querySelectorAll("a"));
        var l = links.length;
        for (var i = 0; i < l; ++i) {
            var link = links[i];
            var href = link.getAttribute("href");
            if (href && !href.startsWith("#") && !/^(?:[a-z+]+:)?\/\//.test(href)) {
                link.href = path_to_root + href;
            }
            // The "index" page is supposed to alias the first chapter in the book.
            if (link.href === current_page || (i === 0 && path_to_root === "" && current_page.endsWith("/index.html"))) {
                link.classList.add("active");
                var parent = link.parentElement;
                if (parent && parent.classList.contains("chapter-item")) {
                    parent.classList.add("expanded");
                }
                while (parent) {
                    if (parent.tagName === "LI" && parent.previousElementSibling) {
                        if (parent.previousElementSibling.classList.contains("chapter-item")) {
                            parent.previousElementSibling.classList.add("expanded");
                        }
                    }
                    parent = parent.parentElement;
                }
            }
        }
        // Track and set sidebar scroll position
        this.addEventListener('click', function(e) {
            if (e.target.tagName === 'A') {
                sessionStorage.setItem('sidebar-scroll', this.scrollTop);
            }
        }, { passive: true });
        var sidebarScrollTop = sessionStorage.getItem('sidebar-scroll');
        sessionStorage.removeItem('sidebar-scroll');
        if (sidebarScrollTop) {
            // preserve sidebar scroll position when navigating via links within sidebar
            this.scrollTop = sidebarScrollTop;
        } else {
            // scroll sidebar to current active section when navigating via "next/previous chapter" buttons
            var activeSection = document.querySelector('#sidebar .active');
            if (activeSection) {
                activeSection.scrollIntoView({ block: 'center' });
            }
        }
        // Toggle buttons
        var sidebarAnchorToggles = document.querySelectorAll('#sidebar a.toggle');
        function toggleSection(ev) {
            ev.currentTarget.parentElement.classList.toggle('expanded');
        }
        Array.from(sidebarAnchorToggles).forEach(function (el) {
            el.addEventListener('click', toggleSection);
        });
    }
}
window.customElements.define("mdbook-sidebar-scrollbox", MDBookSidebarScrollbox);
