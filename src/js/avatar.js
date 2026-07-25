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

    var svg = '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="120" viewBox="0 0 120 120">' +
      '<rect width="120" height="120" fill="#0B1120"/>' +
      '<circle cx="60" cy="60" r="56" fill="rgba(59,244,160,0.08)" stroke="rgba(59,244,160,0.35)" stroke-width="2"/>' +
      '<foreignObject x="0" y="0" width="120" height="120">' +
        '<div xmlns="http://www.w3.org/1999/xhtml" style="width:120px;height:120px;display:flex;align-items:center;justify-content:center;font-family:Chakra Petch, -apple-system, BlinkMacSystemFont, \'Segoe UI\', Roboto, sans-serif;font-size:52px;font-weight:600;color:#3BF4A0;line-height:1;">' + safeInitial + '</div>' +
      '</foreignObject>' +
      '</svg>';

    // UTF-8 safe base64 without deprecated unescape().
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
