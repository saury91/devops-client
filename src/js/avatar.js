// Shared default avatar generator: produces a base64 SVG data URI with the given initial.
var AvatarUtil = (function () {
  'use strict';

  function escapeXml(text) {
    return String(text)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&apos;');
  }

  function generateDefaultAvatar(initial) {
    var safeInitial = initial ? String(initial).trim().charAt(0).toUpperCase() : '?';
    safeInitial = escapeXml(safeInitial);

    // Use native SVG <text> for broad WebView compatibility (no foreignObject).
    var svg = '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="120" viewBox="0 0 120 120">' +
      '<rect width="120" height="120" fill="#0B1120"/>' +
      '<circle cx="60" cy="60" r="56" fill="rgba(59,244,160,0.08)" stroke="rgba(59,244,160,0.35)" stroke-width="2"/>' +
      '<text x="60" y="60" text-anchor="middle" dominant-baseline="central" font-family="Inter, -apple-system, BlinkMacSystemFont, \'Segoe UI\', Roboto, sans-serif" font-size="52" font-weight="600" fill="#3BF4A0">' + safeInitial + '</text>' +
      '</svg>';

    var bytes = new TextEncoder().encode(svg);
    var binary = '';
    for (var i = 0; i < bytes.length; i++) {
      binary += String.fromCharCode(bytes[i]);
    }
    return 'data:image/svg+xml;base64,' + btoa(binary);
  }

  return {
    generateDefaultAvatar: generateDefaultAvatar
  };
})();
