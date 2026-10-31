// Points the hero's download button at the visitor's system. Windows is the default
// in the markup. A Mac gets the Apple silicon build. Chrome and Edge can say when the
// Mac is Intel; in other browsers, Intel owners use the link beside the button, which
// goes to the tickets.
(() => {
  const button = document.getElementById("primary-download");
  const other = document.getElementById("other-systems");
  if (!button || !other) return;

  const platform = (navigator.userAgentData?.platform || navigator.platform || "").toLowerCase();
  const agent = navigator.userAgent.toLowerCase();
  const isMac = platform.startsWith("mac") || (agent.includes("mac os") && !agent.includes("iphone") && !agent.includes("ipad"));
  const isLinux = !isMac && (platform.includes("linux") || agent.includes("linux")) && !agent.includes("android");

  const label = button.querySelector("span");
  if (isMac) {
    button.href = button.dataset.mac;
    label.textContent = "Download for Mac";
    other.textContent = "Intel Mac and other systems";
    navigator.userAgentData
      ?.getHighEntropyValues(["architecture"])
      .then(({ architecture }) => {
        if (architecture === "x86") {
          button.href = button.dataset.macIntel;
          label.textContent = "Download for Intel Mac";
          other.textContent = "Apple silicon and other systems";
        }
      })
      .catch(() => {});
  } else if (isLinux) {
    button.href = button.dataset.linux;
    label.textContent = "Download for Linux";
    other.textContent = button.href.endsWith(".AppImage") ? "Other Linux packages and systems" : "Windows and macOS";
  }
})();
