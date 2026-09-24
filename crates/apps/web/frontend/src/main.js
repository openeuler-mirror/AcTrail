import { createApp } from 'vue';

import App from './App.vue';
import { REVEAL_STYLE, vReveal } from './composables/reveal';
import './styles.css';

const app = createApp(App);
app.directive('reveal', vReveal);

const revealStyle = document.createElement('style');
revealStyle.id = 'actrail-reveal';
revealStyle.textContent = REVEAL_STYLE;
document.head.appendChild(revealStyle);

app.mount('#app');
