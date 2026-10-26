document.querySelector('#location').textContent = LAB.delivery;
document.querySelector('#enable').onclick = async () => {
  const result = await chrome.runtime.sendMessage({type: 'authorize',
    takeoverConsent: document.querySelector('#consent').checked, saveLocationConsent: LAB.delivery});
  document.querySelector('#state').textContent = JSON.stringify(result, null, 2);
};
