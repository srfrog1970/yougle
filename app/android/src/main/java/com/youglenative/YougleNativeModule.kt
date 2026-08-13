package com.youglenative

import com.facebook.react.bridge.ReactApplicationContext

class YougleNativeModule(reactContext: ReactApplicationContext) :
  NativeYougleNativeSpec(reactContext) {

  override fun multiply(a: Double, b: Double): Double {
    return a * b
  }

  companion object {
    const val NAME = NativeYougleNativeSpec.NAME
  }
}
