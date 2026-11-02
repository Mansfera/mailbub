package com.mansfera.mailbub

import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // The app always draws edge to edge: keep the page clear of the system bars, the camera
    // cutout and the keyboard. The strips this leaves show the theme's window background.
    val sides =
      WindowInsetsCompat.Type.systemBars() or
        WindowInsetsCompat.Type.displayCutout() or
        WindowInsetsCompat.Type.ime()
    ViewCompat.setOnApplyWindowInsetsListener(findViewById<View>(android.R.id.content)) { view, insets ->
      val bars = insets.getInsets(sides)
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }
}
