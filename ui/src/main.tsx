import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import { initToken } from './api/token'
import './index.scss'

// Must run before anything calls the API.
initToken()

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
